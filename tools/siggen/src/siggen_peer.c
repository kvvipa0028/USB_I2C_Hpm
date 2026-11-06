#include "siggen_peer.h"
#include <inttypes.h>
#include <stdio.h>
#include <string.h>

enum { SLAVE_IDLE, SLAVE_RX, SLAVE_TX };
enum { MASTER_NONE, MASTER_WAIT_WRITE, MASTER_WRITE, MASTER_WAIT_READ, MASTER_READ };
#define ROUND_PERIOD_US UINT64_C(100000)
#define ROUND_TIMEOUT_US UINT64_C(500000)

const char *siggen_result_name(enum siggen_result r)
{
    static const char *const names[] = { "pending", "ok", "address_nack", "data_nack",
        "timeout", "error", "cancelled", "not_run" };
    return (unsigned)r < sizeof(names) / sizeof(names[0]) ? names[r] : "invalid";
}
static bool active(const siggen_peer_t *p)
{
    return p->master_phase != MASTER_NONE || p->slave_state != SLAVE_IDLE ||
        p->pending_ack || p->held_mask != 0 || p->hal.busy(p->hal.context);
}
static bool configure(siggen_peer_t *p)
{
    if (p->mode == SIG_MODE_IDLE) { p->hal.disable(p->hal.context); return true; }
    return p->hal.configure(p->hal.context, p->mode == SIG_MODE_SLAVE,
                           p->address, p->speed_hz, p->address_nack);
}
static void cancel(siggen_peer_t *p)
{
    if (p->pending_ack) ++p->cancelled_acks;
    p->pending_ack = false;
    p->hal.disable(p->hal.context);
    p->hal.unhold(p->hal.context);
    p->held_mask = 0;
    p->slave_state = SLAVE_IDLE;
    if (p->master_phase != MASTER_NONE) {
        if (p->write_result.result == SIG_RESULT_PENDING)
            p->write_result = (siggen_result_t){ SIG_RESULT_CANCELLED, 0 };
        if (p->read_result.result == SIG_RESULT_PENDING)
            p->read_result = (siggen_result_t){ SIG_RESULT_CANCELLED, 0 };
    }
    p->master_phase = MASTER_NONE;
    p->running = false;
    p->stretch_us = 0;
    p->nack_data = 0;
    p->address_nack = false;
}
void siggen_peer_stop(siggen_peer_t *p)
{
    cancel(p);
    p->mode = SIG_MODE_IDLE;
}
void siggen_peer_init(siggen_peer_t *p, const siggen_hal_t *hal)
{
    memset(p, 0, sizeof(*p));
    p->hal = *hal;
    p->address = 0x50;
    p->speed_hz = 100000;
    p->write_result.result = SIG_RESULT_NOT_RUN;
    p->read_result.result = SIG_RESULT_NOT_RUN;
    siggen_peer_stop(p);
}
static void round_done(siggen_peer_t *p, uint64_t now)
{
    ++p->rounds;
    p->master_phase = MASTER_NONE;
    p->last_round = now;
    /* Reset the selected controller to release any failed transaction. */
    if (!configure(p)) {
        p->running = false; p->mode = SIG_MODE_IDLE;
        p->hal.disable(p->hal.context);
    }
}
void siggen_peer_poll(siggen_peer_t *p, uint64_t now)
{
    if (p->last_poll != 0 && now - p->last_poll > p->max_poll_gap_us)
        p->max_poll_gap_us = now - p->last_poll;
    p->last_poll = now;
    if (p->held_mask != 0) {
        if (now - p->hold_started >= p->hold_duration_us) {
            p->hal.unhold(p->hal.context);
            p->held_mask = 0;
            if (!configure(p)) { p->mode = SIG_MODE_IDLE; p->hal.disable(p->hal.context); }
        }
        return;
    }
    if (p->pending_ack) {
        if (!p->hal.ack_valid(p->hal.context)) {
            p->pending_ack = false;
            ++p->cancelled_acks;
            p->slave_state = SLAVE_IDLE;
            if (!configure(p)) { p->mode = SIG_MODE_IDLE; p->hal.disable(p->hal.context); }
        } else if (now - p->ack_started >= p->stretch_us) {
            p->last_stretch_us = now - p->ack_started;
            if (p->last_stretch_us > p->max_stretch_us) p->max_stretch_us = p->last_stretch_us;
            p->hal.ack(p->hal.context, true);
            ++p->ack_count;
            p->pending_ack = false;
        }
    }
    if (p->mode != SIG_MODE_MASTER || !p->running) return;
    if (p->master_phase == MASTER_NONE && now - p->last_round >= ROUND_PERIOD_US) {
        p->round_started = now;
        p->master_phase = MASTER_WAIT_WRITE;
        p->write_result = (siggen_result_t){ SIG_RESULT_PENDING, 0 };
        p->read_result = (siggen_result_t){ SIG_RESULT_NOT_RUN, 0 };
    }
    if (p->master_phase == MASTER_NONE) return;
    if (now - p->round_started >= ROUND_TIMEOUT_US) {
        if (p->master_phase <= MASTER_WRITE) p->write_result = (siggen_result_t){ SIG_RESULT_TIMEOUT, 0 };
        else p->read_result = (siggen_result_t){ SIG_RESULT_TIMEOUT, 0 };
        round_done(p, now);
        return;
    }
    if (p->master_phase == MASTER_WAIT_WRITE || p->master_phase == MASTER_WAIT_READ) {
        bool read = p->master_phase == MASTER_WAIT_READ;
        if (p->hal.master_begin(p->hal.context, read, p->address)) {
            p->master_phase = read ? MASTER_READ : MASTER_WRITE;
            if (read) p->read_result = (siggen_result_t){ SIG_RESULT_PENDING, 0 };
        }
        return;
    }
    siggen_result_t result;
    bool read = p->master_phase == MASTER_READ;
    if (!p->hal.master_poll(p->hal.context, read, &result, p->last_read)) return;
    if (!read) {
        p->write_result = result;
        if (result.result == SIG_RESULT_ERROR) { round_done(p, now); return; }
        p->master_phase = MASTER_WAIT_READ;
    } else {
        p->read_result = result;
        round_done(p, now);
    }
}
void siggen_peer_address(siggen_peer_t *p, bool master_read)
{
    if (p->mode != SIG_MODE_SLAVE || p->address_nack || p->held_mask != 0) return;
    if (p->pending_ack) { p->pending_ack = false; ++p->cancelled_acks; }
    ++p->address_hits;
    if (master_read) {
        p->slave_state = SLAVE_TX;
        p->tx_start = p->offset;
        p->tx_next = p->offset;
    } else {
        p->slave_state = SLAVE_RX;
        p->rx_first = true;
        p->write_index = 0;
        p->rejected = false;
    }
}
enum siggen_ack siggen_peer_receive(siggen_peer_t *p, uint8_t value, uint64_t now)
{
    if (p->mode != SIG_MODE_SLAVE || p->slave_state != SLAVE_RX || p->pending_ack) {
        ++p->nack_count;
        return SIG_NACK_NOW;
    }
    ++p->rx_bytes;
    if (p->write_index < UINT16_MAX) ++p->write_index;
    if (p->rejected || (p->nack_data != 0 && p->write_index == p->nack_data)) {
        p->rejected = true;
        ++p->nack_count;
        return SIG_NACK_NOW;
    }
    if (p->rx_first) { p->offset = value; p->rx_first = false; }
    else p->table[p->offset++] = value;
    if (p->write_index == 1 && p->stretch_us != 0) {
        p->pending_ack = true;
        p->ack_started = now;
        p->ack_deadline = now + p->stretch_us;
        ++p->stretch_count;
        return SIG_ACK_DEFER;
    }
    ++p->ack_count;
    return SIG_ACK_NOW;
}
uint8_t siggen_peer_transmit(siggen_peer_t *p)
{
    return p->table[p->tx_next++];
}
void siggen_peer_complete(siggen_peer_t *p, uint16_t actual_tx_count)
{
    if (p->pending_ack) { p->pending_ack = false; ++p->cancelled_acks; }
    if (p->slave_state == SLAVE_TX) {
        p->offset = (uint8_t)(p->tx_start + actual_tx_count);
        p->tx_bytes += actual_tx_count;
        ++p->tx_transactions;
    } else if (p->slave_state == SLAVE_RX) ++p->rx_transactions;
    p->slave_state = SLAVE_IDLE;
}
static void format_status(const siggen_reply_t *reply, char *out, size_t cap)
{
    const siggen_peer_t *p = &reply->snapshot;
    const char *mode = p->mode == SIG_MODE_MASTER ? "master" : p->mode == SIG_MODE_SLAVE ? "slave" : "idle";
    (void)snprintf(out, cap,
        "OK running=%u rounds=%" PRIu32 " mode=%s addr=0x%02x hz=%" PRIu32
        " busy=%u lines=%u hold=%u ack_pending=%u stretch_us=%" PRIu32
        " ack_deadline_us=%" PRIu64 " last_stretch_us=%" PRIu64 " max_stretch_us=%" PRIu64
        " poll_gap_max_us=%" PRIu64 " nack_address=%u nack_data=%" PRIu32
        " address_hits=%" PRIu32 " rx_transactions=%" PRIu32 " tx_transactions=%" PRIu32
        " rx_bytes=%" PRIu32 " tx_datacnt_mod256_sum=%" PRIu32 " tx_count_bits=8 ack=%" PRIu32 " nack=%" PRIu32
        " stretch_count=%" PRIu32 " hold_count=%" PRIu32 " cancelled_ack=%" PRIu32
        " write_status=%s write_raw=%" PRIu32 " read_status=%s read_raw=%" PRIu32
        " read_hex=%02x%02x\n",
        p->running ? 1U : 0U, p->rounds, mode, (unsigned)p->address, p->speed_hz,
        reply->busy ? 1U : 0U, (unsigned)reply->lines, (unsigned)p->held_mask,
        p->pending_ack ? 1U : 0U, p->stretch_us, p->ack_deadline, p->last_stretch_us,
        p->max_stretch_us, p->max_poll_gap_us, p->address_nack ? 1U : 0U, p->nack_data,
        p->address_hits, p->rx_transactions, p->tx_transactions, p->rx_bytes, p->tx_bytes,
        p->ack_count, p->nack_count, p->stretch_count, p->hold_count, p->cancelled_acks,
        siggen_result_name(p->write_result.result), p->write_result.raw,
        siggen_result_name(p->read_result.result), p->read_result.raw,
        (unsigned)p->last_read[0], (unsigned)p->last_read[1]);
}
void siggen_peer_prepare(siggen_peer_t *p, const siggen_command_t *c, uint64_t now,
                         siggen_reply_t *reply)
{
    reply->kind = SIG_REPLY_NONE;
    reply->text = NULL;
    if (c->kind == SIG_CMD_STATUS) {
        reply->kind = SIG_REPLY_STATUS;
        /* Status never reads the table. Keep the locked copy bounded to the
         * state/counters prefix, and format without any later live reads. */
        memcpy(&reply->snapshot, p, offsetof(siggen_peer_t, table));
        reply->busy = active(p);
        reply->lines = p->hal.lines(p->hal.context);
        return;
    }
    if (c->kind == SIG_CMD_HELP) {
        reply->kind = SIG_REPLY_TEXT;
        reply->text = "OK start | stop | status | master/slave addr hz | stretch us (first write byte only) | nack address 0/1 | nack data index | hold scl/sda/both ms | release | table read off len<=32 | table fill byte | table write off hex<=32B\n";
        return;
    }
    if (c->kind == SIG_CMD_STOP) { siggen_peer_stop(p); reply->kind = SIG_REPLY_TEXT; reply->text = "OK stop\n"; return; }
    if (c->kind == SIG_CMD_RELEASE) {
        cancel(p);
        bool ok = configure(p);
        if (!ok) { p->mode = SIG_MODE_IDLE; p->hal.disable(p->hal.context); }
        reply->kind = SIG_REPLY_TEXT;
        reply->text = ok ? "OK release\n" : "ERR configure\n";
        return;
    }
    if (active(p)) { reply->kind = SIG_REPLY_TEXT; reply->text = "ERR busy\n"; return; }
    switch (c->kind) {
    case SIG_CMD_START:
        cancel(p); p->mode = SIG_MODE_MASTER;
        if (!configure(p)) goto failed;
        p->running = true; p->last_round = now;
        reply->kind = SIG_REPLY_TEXT; reply->text = "OK start\n"; return;
    case SIG_CMD_MASTER: case SIG_CMD_SLAVE:
        cancel(p);
        p->mode = c->kind == SIG_CMD_SLAVE ? SIG_MODE_SLAVE : SIG_MODE_MASTER;
        p->address = (uint8_t)c->value[0]; p->speed_hz = c->value[1]; p->offset = 0;
        if (!configure(p)) goto failed;
        break;
    case SIG_CMD_STRETCH:
        if (p->mode != SIG_MODE_SLAVE) goto role;
        p->stretch_us = c->value[0]; break;
    case SIG_CMD_NACK_ADDRESS:
        if (p->mode != SIG_MODE_SLAVE) goto role;
        p->address_nack = c->value[0] != 0;
        if (!configure(p)) goto failed;
        break;
    case SIG_CMD_NACK_DATA:
        if (p->mode != SIG_MODE_SLAVE) goto role;
        p->nack_data = c->value[0]; break;
    case SIG_CMD_HOLD:
        p->running = false;
        p->held_mask = (uint8_t)c->value[0];
        p->hold_started = now;
        p->hold_duration_us = (uint64_t)c->value[1] * 1000U;
        p->hal.hold(p->hal.context, p->held_mask);
        ++p->hold_count; break;
    case SIG_CMD_TABLE_FILL:
        memset(p->table, (int)c->value[0], sizeof(p->table)); break;
    case SIG_CMD_TABLE_WRITE:
        memcpy(p->table + c->value[0], c->data, c->data_len); break;
    case SIG_CMD_TABLE_READ:
        reply->kind = SIG_REPLY_TABLE;
        reply->table_offset = c->value[0];
        reply->table_length = c->value[1];
        memcpy(reply->table, p->table + c->value[0], c->value[1]);
        return;
    default: reply->kind = SIG_REPLY_TEXT; reply->text = "ERR command\n"; return;
    }
    reply->kind = SIG_REPLY_TEXT; reply->text = "OK\n"; return;
role:
    reply->kind = SIG_REPLY_TEXT; reply->text = "ERR slave required\n"; return;
failed:
    siggen_peer_stop(p);
    reply->kind = SIG_REPLY_TEXT; reply->text = "ERR configure\n";
}

void siggen_peer_format(const siggen_reply_t *reply, char *out, size_t cap)
{
    if (cap == 0U || reply == NULL || out == NULL) return;
    if (reply->kind == SIG_REPLY_TEXT) {
        (void)snprintf(out, cap, "%s", reply->text == NULL ? "ERR command\n" : reply->text);
    } else if (reply->kind == SIG_REPLY_STATUS) {
        format_status(reply, out, cap);
    } else if (reply->kind == SIG_REPLY_TABLE) {
        size_t used = (size_t)snprintf(out, cap, "OK table offset=%" PRIu32 " length=%" PRIu32 " hex=", reply->table_offset, reply->table_length);
        for (uint32_t i = 0U; i < reply->table_length && used < cap; ++i)
            used += (size_t)snprintf(out + used, cap - used, "%02x", (unsigned)reply->table[i]);
        if (used < cap) (void)snprintf(out + used, cap - used, "\n");
    } else {
        out[0] = '\0';
    }
}

void siggen_peer_command(siggen_peer_t *p, const siggen_command_t *c, uint64_t now,
                         char *out, size_t cap)
{
    siggen_reply_t reply;
    if (cap == 0U) return;
    siggen_peer_prepare(p, c, now, &reply);
    siggen_peer_format(&reply, out, cap);
}
