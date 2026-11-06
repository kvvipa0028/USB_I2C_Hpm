#include "siggen_session.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

typedef struct {
    bool configured, config_ok, busy, ack_valid, complete, configured_slave, address_nack;
    uint8_t low_mask, lines, configured_address;
    uint32_t config_count, disables, unholds, holds, acks, nacks, starts, configured_hz, line_calls;
    siggen_result_t result;
} fake_t;
static siggen_peer_t p;
static fake_t f;
static char response[SIGGEN_RESPONSE_CAP];
static bool config(void *ctx, bool slave, uint8_t addr, uint32_t hz, bool nack)
{
    fake_t *h = ctx; ++h->config_count; h->configured_slave = slave;
    h->configured_address = addr; h->configured_hz = hz; h->address_nack = nack;
    h->configured = h->config_ok; h->busy = !h->config_ok;
    return h->config_ok;
}
static void disable(void *ctx) { fake_t *h = ctx; ++h->disables; h->configured = false; h->busy = false; }
static bool busy(void *ctx) { return ((fake_t *)ctx)->busy; }
static void hold(void *ctx, uint8_t mask) { fake_t *h = ctx; ++h->holds; h->low_mask = mask; }
static void unhold(void *ctx) { fake_t *h = ctx; ++h->unholds; h->low_mask = 0; }
static uint8_t lines(void *ctx) { fake_t *h = ctx; ++h->line_calls; return h->lines; }
static void ack(void *ctx, bool yes) { fake_t *h = ctx; if (yes) ++h->acks; else ++h->nacks; }
static bool ack_valid(void *ctx) { return ((fake_t *)ctx)->ack_valid; }
static bool begin(void *ctx, bool read, uint8_t addr)
{
    fake_t *h = ctx; (void)read; assert(addr == p.address);
    if (h->busy) return false;
    ++h->starts; return true;
}
static bool poll(void *ctx, bool read, siggen_result_t *r, uint8_t data[2])
{
    fake_t *h = ctx; (void)read; if (!h->complete) return false;
    *r = h->result; data[0] = 0x11; data[1] = 0x22; return true;
}
static void setup(void)
{
    memset(&f, 0, sizeof(f)); f.config_ok = true; f.ack_valid = true; f.lines = 3;
    siggen_hal_t hal = { &f, config, disable, busy, hold, unhold, lines, ack,
        ack_valid, begin, poll };
    siggen_peer_init(&p, &hal);
    assert(p.mode == SIG_MODE_IDLE && !p.running && f.low_mask == 0);
}
static void cmd(const char *text, uint64_t now, const char *expected)
{
    siggen_command_t c; assert(siggen_command_parse(text, &c));
    memset(response, 0x7f, sizeof(response));
    siggen_peer_command(&p, &c, now, response, sizeof(response));
    assert(strlen(response) < sizeof(response) - 1U);
    assert(response[strlen(response) - 1U] == '\n');
    assert(strncmp(response, expected, strlen(expected)) == 0);
}
static void table_tests(void)
{
    setup(); cmd("slave 0x50 100000", 0, "OK");
    assert(f.configured_slave && f.configured_hz == 100000);
    cmd("table fill 0x99", 0, "OK");
    cmd("table write 254 aabb", 0, "OK");
    assert(p.table[253] == 0x99 && p.table[254] == 0xaa && p.table[255] == 0xbb);
    cmd("table read 254 2", 0, "OK table offset=254 length=2 hex=aabb\n");
    siggen_peer_address(&p, false);
    assert(siggen_peer_receive(&p, 254, 0) == SIG_ACK_NOW);
    assert(siggen_peer_receive(&p, 0x11, 1) == SIG_ACK_NOW);
    assert(siggen_peer_receive(&p, 0x22, 2) == SIG_ACK_NOW);
    assert(siggen_peer_receive(&p, 0x33, 3) == SIG_ACK_NOW);
    siggen_peer_complete(&p, 0);
    assert(p.table[254] == 0x11 && p.table[255] == 0x22 && p.table[0] == 0x33);
    siggen_peer_address(&p, false); assert(siggen_peer_receive(&p, 254, 0) == SIG_ACK_NOW);
    /* Repeated START retains offset. FIFO prefetch is not bytes sent. */
    siggen_peer_address(&p, true);
    assert(siggen_peer_transmit(&p) == 0x11); assert(siggen_peer_transmit(&p) == 0x22);
    assert(siggen_peer_transmit(&p) == 0x33); (void)siggen_peer_transmit(&p);
    siggen_peer_complete(&p, 2); assert(p.offset == 0 && p.tx_bytes == 2);
    cmd("master 0x51 400000", 0, "OK"); assert(p.table[254] == 0x11);
}
static void nack_tests(void)
{
    setup(); cmd("slave 0x50 100000", 0, "OK"); cmd("nack address 1", 0, "OK");
    assert(f.address_nack); siggen_peer_address(&p, false); assert(p.address_hits == 0);
    cmd("nack address 0", 0, "OK"); cmd("nack data 2", 0, "OK");
    siggen_peer_address(&p, false); assert(siggen_peer_receive(&p, 5, 0) == SIG_ACK_NOW);
    assert(siggen_peer_receive(&p, 0x33, 0) == SIG_NACK_NOW && p.table[5] == 0);
    assert(siggen_peer_receive(&p, 0x44, 0) == SIG_NACK_NOW && p.table[5] == 0);
    siggen_peer_complete(&p, 0); cmd("nack data 256", 0, "OK");
    siggen_peer_address(&p, false); assert(siggen_peer_receive(&p, 0, 0) == SIG_ACK_NOW);
    for (unsigned i = 0; i < 254; ++i) assert(siggen_peer_receive(&p, 0xaa, 0) == SIG_ACK_NOW);
    assert(siggen_peer_receive(&p, 0xbb, 0) == SIG_NACK_NOW && p.table[254] == 0);
    siggen_peer_complete(&p, 0); cmd("release", 0, "OK"); assert(p.nack_data == 0 && !p.address_nack);
}
static void arm_stretch(uint64_t now)
{
    setup(); cmd("slave 0x50 100000", now, "OK"); cmd("stretch 10000", now, "OK");
    siggen_peer_address(&p, false);
    assert(siggen_peer_receive(&p, 0, now) == SIG_ACK_DEFER && p.pending_ack);
}
static void stretch_tests(void)
{
    arm_stretch(100); siggen_peer_poll(&p, 10099); assert(f.acks == 0 && p.pending_ack);
    siggen_peer_poll(&p, 10100); assert(f.acks == 1 && !p.pending_ack && p.last_stretch_us == 10000);
    assert(siggen_peer_receive(&p, 0x55, 10101) == SIG_ACK_NOW && p.stretch_count == 1);
    siggen_peer_complete(&p, 0);
    cmd("stretch 0", 0, "OK"); siggen_peer_address(&p, false);
    assert(siggen_peer_receive(&p, 0, 0) == SIG_ACK_NOW && !p.pending_ack);
    arm_stretch(UINT64_MAX - 999U); siggen_peer_poll(&p, 8999); assert(f.acks == 0);
    siggen_peer_poll(&p, 9000); assert(f.acks == 1 && p.last_stretch_us == 10000);
    arm_stretch(0); siggen_peer_poll(&p, 15000); assert(p.last_stretch_us == 15000);
    /* Invalidated hardware completion cancels instead of issuing a late ACK. */
    arm_stretch(0); f.ack_valid = false; siggen_peer_poll(&p, 10000);
    assert(f.acks == 0 && !p.pending_ack && p.cancelled_acks == 1);
    const char *cancel_commands[] = { "stop", "release" };
    for (unsigned i = 0; i < 2; ++i) {
        arm_stretch(0); cmd(cancel_commands[i], 1, "OK"); siggen_peer_poll(&p, 20000);
        assert(f.acks == 0 && !p.pending_ack && !p.running && f.low_mask == 0);
    }
    arm_stretch(0); siggen_peer_complete(&p, 0); siggen_peer_poll(&p, 20000); assert(f.acks == 0);
    arm_stretch(0); siggen_peer_address(&p, true); siggen_peer_poll(&p, 20000); assert(f.acks == 0);
    arm_stretch(0); cmd("master 0x50 100000", 1, "ERR busy"); assert(p.pending_ack);
    siggen_peer_stop(&p); siggen_peer_poll(&p, 20000); assert(f.acks == 0 && !p.running);
}
static void hold_tests(void)
{
    setup(); cmd("slave 0x52 400000", 0, "OK"); cmd("hold sda 20", 10, "OK");
    assert(f.low_mask == 2 && p.held_mask == 2);
    cmd("start", 11, "ERR busy"); cmd("slave 0x50 100000", 11, "ERR busy");
    siggen_peer_poll(&p, 20009); assert(f.low_mask == 2);
    siggen_peer_poll(&p, 20010); assert(f.low_mask == 0 && p.held_mask == 0 && f.configured_slave);
    cmd("hold both 5", UINT64_MAX - 99U, "OK");
    siggen_peer_poll(&p, 4899); assert(f.low_mask == 3);
    siggen_peer_poll(&p, 4900); assert(f.low_mask == 0);
    cmd("hold scl 5000", 0, "OK"); cmd("release", 1, "OK"); cmd("release", 2, "OK");
    assert(f.low_mask == 0 && p.mode == SIG_MODE_SLAVE && !p.running);
    cmd("hold both 5000", 0, "OK"); siggen_peer_stop(&p); assert(f.low_mask == 0 && p.mode == SIG_MODE_IDLE);
}
static void master_tests(void)
{
    setup(); cmd("start", 0, "OK start\n"); siggen_peer_poll(&p, 99999); assert(f.starts == 0);
    siggen_peer_poll(&p, 100000); assert(f.starts == 1);
    f.complete = true; f.result = (siggen_result_t){ SIG_RESULT_ADDRESS_NACK, 2003 };
    siggen_peer_poll(&p, 100001); assert(p.write_result.raw == 2003);
    siggen_peer_poll(&p, 100002); assert(f.starts == 2);
    f.result = (siggen_result_t){ SIG_RESULT_OK, 0 }; siggen_peer_poll(&p, 100003);
    assert(p.rounds == 1 && p.read_result.result == SIG_RESULT_OK && p.last_read[1] == 0x22);
    siggen_peer_poll(&p, 200002); assert(f.starts == 2);
    cmd("stop", 200002, "OK stop\n"); siggen_peer_poll(&p, 900000); assert(f.starts == 2);
    /* External bus busy is bounded, not an SDK polling loop. */
    setup(); cmd("start", 0, "OK"); f.busy = true;
    siggen_peer_poll(&p, 100000); siggen_peer_poll(&p, 599999); assert(p.rounds == 0);
    siggen_peer_poll(&p, 600000); assert(p.rounds == 1 && p.write_result.result == SIG_RESULT_TIMEOUT);
    /* Reconfigure failure must still disable/release, preserving error. */
    setup(); cmd("start", 0, "OK"); siggen_peer_poll(&p, 100000); f.config_ok = false;
    uint32_t before = f.disables; siggen_peer_poll(&p, 600000);
    assert(f.disables == before + 1U && !f.busy && p.mode == SIG_MODE_IDLE && !p.running);
    assert(p.write_result.result == SIG_RESULT_TIMEOUT); f.config_ok = true;
    cmd("slave 0x50 100000", 600001, "OK");
}
static void session_tests(void)
{
    siggen_line_assembler_t line; siggen_line_assembler_init(&line);
    arm_stretch(0);
    assert(siggen_session_feed(&p, &line, -2, 1, response));
    siggen_peer_poll(&p, 20000); assert(f.acks == 0 && p.mode == SIG_MODE_IDLE);
    arm_stretch(0); siggen_line_assembler_init(&line);
    for (unsigned i = 0; i < SIGGEN_LINE_CAP; ++i) assert(!siggen_session_feed(&p, &line, 's', 1, response));
    assert(p.mode == SIG_MODE_IDLE && !p.pending_ack);
    assert(siggen_session_feed(&p, &line, '\n', 1, response));
    assert(strstr(response, "ERR malformed") != NULL);
    siggen_peer_poll(&p, 20000); assert(f.acks == 0);
    setup(); siggen_line_assembler_init(&line);
    const char *text = "start\nstop\r\n"; unsigned replies = 0;
    for (size_t i = 0; text[i] != 0; ++i) if (siggen_session_feed(&p, &line, text[i], 0, response)) ++replies;
    assert(replies == 2 && p.mode == SIG_MODE_IDLE);
    /* Max-width telemetry still fits one complete CDC response. */
    p.rounds = p.address_hits = p.rx_transactions = p.tx_transactions = UINT32_MAX;
    p.rx_bytes = p.tx_bytes = p.ack_count = p.nack_count = UINT32_MAX;
    p.stretch_count = p.hold_count = p.cancelled_acks = UINT32_MAX;
    p.ack_deadline = p.last_stretch_us = p.max_stretch_us = p.max_poll_gap_us = UINT64_MAX;
    cmd("status", 0, "OK running="); assert(strstr(response, "read_hex=") != NULL);
    assert(strstr(response, "tx_datacnt_mod256_sum=4294967295 tx_count_bits=8") != NULL);
    assert(strstr(response, " tx_bytes=") == NULL);
}
static void reply_snapshot_tests(void)
{
    siggen_command_t c;
    siggen_reply_t reply;
    char out[SIGGEN_RESPONSE_CAP];
    setup();
    assert(siggen_command_parse("slave 0x50 100000", &c));
    siggen_peer_prepare(&p, &c, 0, &reply);
    assert(reply.kind == SIG_REPLY_TEXT);
    siggen_peer_format(&reply, out, sizeof(out));
    assert(strcmp(out, "OK\n") == 0);
    assert(siggen_command_parse("table fill 0xaa", &c));
    siggen_peer_prepare(&p, &c, 0, &reply);
    assert(siggen_command_parse("table read 0 2", &c));
    siggen_peer_prepare(&p, &c, 0, &reply);
    p.table[0] = 0x11; p.table[1] = 0x22;
    siggen_peer_format(&reply, out, sizeof(out));
    assert(strcmp(out, "OK table offset=0 length=2 hex=aaaa\n") == 0);

    assert(siggen_command_parse("status", &c));
    f.lines = 1U; f.line_calls = 0U;
    p.rounds = 42U;
    siggen_peer_prepare(&p, &c, 0, &reply);
    assert(f.line_calls == 1U);
    f.lines = 2U;
    p.mode = SIG_MODE_IDLE; p.rounds = 0;
    siggen_peer_format(&reply, out, sizeof(out));
    assert(strstr(out, "lines=1") != NULL);
    assert(strstr(out, "rounds=42 mode=slave") != NULL);
    assert(f.line_calls == 1U);
    memset(out, 0x7f, sizeof(out));
    siggen_peer_format(&reply, out, 32U);
    assert(out[31] == '\0');
}
int main(void)
{
    table_tests(); nack_tests(); stretch_tests(); hold_tests(); master_tests(); session_tests(); reply_snapshot_tests();
    puts("siggen peer/session tests passed");
    return 0;
}
