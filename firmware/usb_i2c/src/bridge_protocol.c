#include "bridge_protocol.h"
#include <string.h>
enum { PENDING_NONE, PENDING_TRANSFER, PENDING_RECOVER };
static uint16_t get16(const uint8_t *b) { return (uint16_t)(b[0] | ((uint16_t)b[1] << 8)); }
static uint32_t get32(const uint8_t *b) { return get16(b) | ((uint32_t)get16(b + 2) << 16); }
static uint64_t get64(const uint8_t *b) { return get32(b) | ((uint64_t)get32(b + 4) << 32); }
static void put16(uint8_t *b, uint16_t n) { b[0] = (uint8_t)n; b[1] = (uint8_t)(n >> 8); }
static void put32(uint8_t *b, uint32_t n) { put16(b, (uint16_t)n); put16(b + 2, (uint16_t)(n >> 16)); }
static void put64(uint8_t *b, uint64_t n) { put32(b, (uint32_t)n); put32(b + 4, (uint32_t)(n >> 32)); }
static bool zeros(const uint8_t *b, unsigned len)
{
    for (unsigned i = 0; i < len; ++i) if (b[i]) return false;
    return true;
}
static void finish(bridge_protocol_t *p, uint8_t status, uint32_t hpm, uint16_t data_len)
{
    p->response[2] = status; put32(p->response + 4, hpm);
    p->response_len = 8U + (status == BRIDGE_OK ? data_len : 0U);
    if (status == BRIDGE_OK && (p->command == 0x12 || p->command == 0x03 || p->command >= 0x13) &&
        p->response_len % 64U == 0) p->response[p->response_len++] = 0;
    p->pending = PENDING_NONE;
}
static void state_data(uint8_t port, uint8_t *b)
{
    bridge_port_state_t s;
    bridge_port_state(port, &s);
    memset(b, 0, 40); b[0] = 1; b[1] = port; b[2] = s.config.role; b[3] = s.activity;
    b[4] = s.config.slave_address; put32(b + 8, s.config.speed_hz); put32(b + 12, s.calculated_hz);
    put32(b + 16, s.config.stretch_us); put32(b + 20, s.config.transaction_us);
    put64(b + 24, s.config_generation); put64(b + 32, s.table_generation);
}
static void table_meta(uint8_t *b, uint8_t port, uint16_t offset, uint16_t len, uint64_t gen)
{
    memset(b, 0, 16); b[0] = port; put16(b + 2, offset); put16(b + 4, len); put64(b + 8, gen);
}
static bool port_query(const uint8_t *b, uint16_t wlen)
{
    return wlen == 4 && b[0] < BRIDGE_PORT_COUNT && zeros(b + 1, 3);
}
static void transfers_start(bridge_protocol_t *p)
{
    /* Validate every item before the first START; a bad second item has no
     * side effect on the first port. Old commands retain status range 0..4. */
    for (uint8_t i = 0; i < p->count; ++i) {
        uint8_t st = bridge_port_transfer_validate(&p->transfer[i]);
        if (st != BRIDGE_OK) {
            finish(p, p->command == 0x17 ? st : BRIDGE_BAD_REQUEST, 0, 0); return;
        }
    }
    for (uint8_t i = 0; i < p->count; ++i) bridge_port_transfer_start(&p->transfer[i]);
    p->pending = PENDING_TRANSFER;
}
void bridge_protocol_start(bridge_protocol_t *p, const uint8_t *req, uint32_t len,
    uint16_t mps, uint8_t *response)
{
    memset(p, 0, sizeof(*p)); p->response = response;
    memset(response, 0, 8); p->response_len = 8; response[2] = BRIDGE_BAD_REQUEST;
    if (len < 8 || len >= 512) return;
    p->command = req[0]; response[0] = req[0]; response[1] = req[1];
    uint16_t wlen = get16(req + 4), rlen = get16(req + 6);
    uint32_t frame = 8U + wlen;
    if (req[3] || (mps != 64 && mps != 512) || frame >= 512) return;
    bool padded = frame % mps == 0;
    if (len != frame + (padded ? 1U : 0U) || (padded && req[frame] != 0)) return;
    const uint8_t *b = req + 8;
    uint8_t *out = response + 8;
    if (p->command == 1 || p->command == 2) {
        if (req[2] || wlen || rlen) return;
        if (p->command == 1) {
            static const char id[] = "usb_i2c 0.2";
            out[0] = 1; memcpy(out + 1, id, sizeof(id) - 1);
            finish(p, BRIDGE_OK, 0, sizeof(id));
        } else {
            memset(out, 0, 20); out[0] = 1; out[1] = BRIDGE_PORT_COUNT; out[2] = 7;
            put16(out + 4, 256); put16(out + 6, 240); put32(out + 8, 100000);
            put32(out + 12, 10000); put32(out + 16, 1000000); finish(p, BRIDGE_OK, 0, 20);
        }
        return;
    }
    if (p->command == 0x10 || p->command == 0x11) {
        uint8_t port = 0; uint32_t speed = 100000; uint16_t prefix = 0;
        if (p->command == 0x11) {
            if (wlen < 8 || !zeros(b + 1, 3)) return;
            port = b[0]; speed = get32(b + 4); prefix = 8;
        }
        p->count = 1;
        p->transfer[0] = (bridge_transfer_t){ port, req[2], wlen - prefix, rlen, speed, b + prefix, p->data[0], false };
        transfers_start(p); return;
    }
    if (req[2]) return;
    if (p->command == 0x12) {
        if (wlen < 22 || b[0] != 2 || b[1]) return;
        uint32_t pos = 2, expected_read = 18;
        for (unsigned i = 0; i < 2; ++i) {
            if (pos + 10U > wlen) return;
            uint16_t w = get16(b + pos + 2), r = get16(b + pos + 4);
            if (w > 240 || r > 240 || pos + 10U + w > wlen) return;
            p->transfer[i] = (bridge_transfer_t){ b[pos], b[pos + 1], w, r, get32(b + pos + 6),
                b + pos + 10, p->data[i], false };
            expected_read += r; pos += 10U + w;
        }
        if (pos != wlen || p->transfer[0].port == p->transfer[1].port || rlen != expected_read) return;
        p->count = 2; transfers_start(p); return;
    }
    if (p->command == 0x03) {
        if (!port_query(b, wlen) || rlen != 48) return;
        memset(out, 0, 48); out[0] = 1; out[1] = b[0]; put32(out + 4, 0x9F);
        put16(out + 8, 256); put16(out + 10, 256); put32(out + 12, 2000);
        put32(out + 16, 1000); put32(out + 20, 100000); put32(out + 24, 25000);
        put32(out + 28, 1000); put32(out + 32, 2000000); put32(out + 36, 500000);
        put32(out + 40, 1500000); finish(p, BRIDGE_OK, 0, 48); return;
    }
    if (p->command == 0x13) {
        if (!port_query(b, wlen) || rlen != 40) return;
        state_data(b[0], out); finish(p, BRIDGE_OK, 0, 40); return;
    }
    if (p->command == 0x14) {
        if (wlen != 32 || rlen != 40 || b[0] != 1 || b[1] >= BRIDGE_PORT_COUNT ||
            (get32(b + 16) & ~1U) || get32(b + 20) || (!(b[16] & 1U) && get64(b + 24))) return;
        bridge_port_config_t config = { b[2], b[3], get32(b + 4), get32(b + 8), get32(b + 12) };
        uint8_t st = bridge_port_configure(b[1], &config, (b[16] & 1U) != 0, get64(b + 24));
        if (st == BRIDGE_OK) state_data(b[1], out);
        finish(p, st, 0, 40); return;
    }
    if (p->command == 0x15 || p->command == 0x16) {
        bool write = p->command == 0x16;
        if (wlen < (write ? 16 : 8) || b[0] >= BRIDGE_PORT_COUNT || get16(b + 6)) return;
        uint16_t offset = get16(b + 2), count = get16(b + 4);
        if (!count || offset >= 256 || count > 256U - offset) return;
        uint64_t gen = 0; uint8_t st;
        if (write) {
            if (wlen != 16U + count || rlen != 16 || (b[1] & ~1U) || (!b[1] && get64(b + 8))) return;
            st = bridge_port_table_write(b[0], offset, count, b + 16, b[1] != 0, get64(b + 8), &gen);
        } else {
            if (wlen != 8 || rlen != 16U + count || b[1]) return;
            st = bridge_port_table_read(b[0], offset, count, out + 16, &gen);
        }
        table_meta(out, b[0], offset, count, gen);
        finish(p, st, 0, write ? 16 : 16U + count); return;
    }
    if (p->command == 0x17) {
        if (wlen != 4 || rlen != 4 || b[0] >= BRIDGE_PORT_COUNT || !zeros(b + 2, 2) || b[1] < 8 || b[1] > 0x77) return;
        bridge_port_state_t s; bridge_port_state(b[0], &s);
        p->count = 1;
        p->transfer[0] = (bridge_transfer_t){ b[0], b[1], 0, 0, s.config.speed_hz, NULL, NULL, true };
        transfers_start(p); return;
    }
    if (p->command == 0x18) {
        if (!port_query(b, wlen) || rlen != 8) return;
        uint8_t st = bridge_port_recover_start(b[0]);
        if (st) { finish(p, st, 0, 0); return; }
        p->transfer[0].port = b[0]; p->pending = PENDING_RECOVER; return;
    }
    if (p->command == 0x19) {
        if (!port_query(b, wlen) || rlen != 24) return;
        finish(p, BRIDGE_UNSUPPORTED, 0, 0); return;
    }
    if (p->command == 0x1A) {
        if (wlen != 12 || rlen != 24 || b[0] >= BRIDGE_PORT_COUNT || b[1] > 3 || b[2] > 2 || b[3]) return;
        finish(p, BRIDGE_UNSUPPORTED, 0, 0);
    }
}
bool bridge_protocol_poll(bridge_protocol_t *p)
{
    if (!p->pending) return true;
    uint8_t *out = p->response + 8;
    if (p->pending == PENDING_RECOVER) {
        bridge_recovery_result_t r;
        if (!bridge_port_recover_result(p->transfer[0].port, &r)) return false;
        out[0] = p->transfer[0].port; out[1] = r.clocks; out[2] = r.initial_lines; out[3] = r.final_lines;
        put32(out + 4, r.reason); finish(p, BRIDGE_OK, 0, 8); return true;
    }
    for (uint8_t i = 0; i < p->count; ++i)
        if (!bridge_port_transfer_result(p->transfer[i].port, &p->result[i])) return false;
    if (p->command == 0x12) {
        out[0] = 2; out[1] = 0; uint16_t pos = 2;
        for (uint8_t i = 0; i < 2; ++i) {
            uint16_t n = p->result[i].status == BRIDGE_OK ? p->transfer[i].read_len : 0;
            out[pos] = p->transfer[i].port; out[pos + 1] = p->result[i].status;
            put16(out + pos + 2, n); put32(out + pos + 4, p->result[i].hpm_status);
            memcpy(out + pos + 8, p->data[i], n);
            /* Preflight bounds two entries to 2 + 2 * (8 + 240) bytes. */
            pos = (uint16_t)(pos + 8U + n);
        }
        finish(p, BRIDGE_OK, 0, pos);
    } else if (p->command == 0x17 && (p->result[0].status == BRIDGE_OK || p->result[0].status == BRIDGE_NACK)) {
        out[0] = p->transfer[0].port; out[1] = p->transfer[0].address;
        out[2] = p->result[0].status == BRIDGE_OK; out[3] = 0; finish(p, BRIDGE_OK, 0, 4);
    } else {
        uint16_t n = p->transfer[0].read_len;
        if (p->result[0].status == BRIDGE_OK) memcpy(out, p->data[0], n);
        finish(p, p->result[0].status, p->result[0].hpm_status, n);
    }
    return true;
}
