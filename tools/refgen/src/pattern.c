#include "pattern.h"

#include <stdio.h>
#include <string.h>

/* ---------------- builder internals ---------------- */

typedef struct {
    const pat_config_t *cfg;
    uint32_t         *segs;
    size_t            seg_cap, seg_n;
    i2cdec_event_t   *evs;
    size_t            ev_cap, ev_n;
    uint64_t          t;      /* current time in cycles */
    uint8_t           scl, sda;
    uint32_t          period; /* full SCL period in cycles */
    uint32_t          tlow, thigh, tsetup, ttrail;
    uint32_t          min_hold;
    uint64_t          busy;
    int               err;
    char             *err_s;
    size_t            err_len;
} pb_t;

static void perr(pb_t *b, const char *m)
{
    if (!b->err && b->err_s && b->err_len) {
        snprintf(b->err_s, b->err_len, "%s", m);
    }
    b->err = -1;
}

static int room(pb_t *b, size_t nsegs)
{
    if (b->seg_n + nsegs > b->seg_cap || b->seg_n + nsegs > PAT_MAX_SEGS) {
        perr(b, "segment buffer overflow / 128 KiB budget exceeded");
        b->err = -2;
        return 0;
    }
    return 1;
}

/* Emit one segment: set (scl,sda) then hold `hold` cycles.
 * Splits oversized holds; clamps to the PIO minimum. */
static void emit_idle_flags(pb_t *b, uint8_t scl, uint8_t sda, uint64_t hold, int is_idle)
{
    if (b->err) {
        return;
    }
    if (hold < PAT_SEG_MIN_HOLD) {
        hold = PAT_SEG_MIN_HOLD;
    }
    while (hold > 0 && room(b, 1)) {
        uint32_t h = (hold > PAT_SEG_MAX_HOLD) ? PAT_SEG_MAX_HOLD : (uint32_t)hold;
        b->segs[b->seg_n++] = PAT_SEG_WORD(scl, sda, h);
        if (h < b->min_hold) {
            b->min_hold = h;
        }
        hold -= h;
        b->t += h;
        if (!is_idle) {
            b->busy += h;
        }
    }
    b->scl = scl;
    b->sda = sda;
}

#define EMIT(b, scl, sda, hold)  emit_idle_flags((b), (scl), (sda), (hold), 0)
#define EMIT_IDLE(b, hold)       emit_idle_flags((b), 1, 1, (hold), 1)

static void add_ev(pb_t *b, const i2cdec_event_t *ev)
{
    if (b->err) {
        return;
    }
    if (b->ev_n >= b->ev_cap) {
        perr(b, "event buffer overflow");
        b->err = -2;
        return;
    }
    b->evs[b->ev_n++] = *ev;
}

static void ev_simple(pb_t *b, uint8_t type, uint64_t t)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = t;
    ev.type = type;
    add_ev(b, &ev);
}

/* -------- waveform primitives (mirror wavegen.c semantics) -------- */

static uint64_t wf_start(pb_t *b)   /* returns START edge t */
{
    uint64_t t = b->t;
    EMIT(b, 1, 0, b->thigh);   /* SDA fall while SCL high */
    EMIT(b, 0, 0, b->ttrail);  /* SCL low (rest of the low phase) */
    return t;
}

static uint64_t wf_restart(pb_t *b)
{
    EMIT(b, 0, 1, b->tsetup);
    EMIT(b, 1, 1, b->thigh);
    {
        uint64_t t = b->t;
        EMIT(b, 1, 0, b->thigh); /* SDA fall while SCL high: RESTART */
        EMIT(b, 0, 0, b->ttrail);
        return t;
    }
}

static uint64_t wf_stop(pb_t *b)
{
    uint64_t t;
    EMIT(b, 0, 0, b->tsetup);
    EMIT(b, 1, 0, b->thigh);
    t = b->t;
    EMIT(b, 1, 1, b->thigh);   /* SDA rise while SCL high: STOP */
    return t;
}

/* One data bit; returns the SCL-rising-edge timestamp.
 * glitch: 0 none, 1 insert configured glitch during this bit's SCL high. */
static uint64_t wf_bit(pb_t *b, uint8_t bit, int glitch)
{
    uint64_t t;
    EMIT(b, 0, bit & 1u, b->tsetup);
    t = b->t;
    if (glitch && b->cfg->glitch_line == PAT_LINE_SCL) {
        /* SCL pulses low in the middle of its high half-period */
        uint32_t w = b->cfg->glitch_cycles;
        uint32_t p1, p2;
        if (w == 0) {
            w = 1;
        }
        p1 = (b->thigh > w) ? (b->thigh - w) / 2 : b->thigh / 2;
        p2 = (b->thigh > w + p1) ? b->thigh - p1 - w : 0;
        EMIT(b, 1, bit & 1u, p1);
        EMIT(b, 0, bit & 1u, w);
        EMIT(b, 1, bit & 1u, p2);
    } else if (glitch && b->cfg->glitch_line == PAT_LINE_SDA) {
        /* SDA pulses to the opposite level in the middle of SCL high */
        uint32_t w = b->cfg->glitch_cycles;
        uint32_t p1, p2;
        uint8_t v = bit & 1u;
        if (w == 0) {
            w = 1;
        }
        p1 = (b->thigh > w) ? (b->thigh - w) / 2 : b->thigh / 2;
        p2 = (b->thigh > w + p1) ? b->thigh - p1 - w : 0;
        EMIT(b, 1, v, p1);
        EMIT(b, 1, v ^ 1u, w);
        EMIT(b, 1, v, p2);
    } else {
        EMIT(b, 1, bit & 1u, b->thigh);
    }
    /* rest of the SCL low phase: total low = tsetup + ttrail = tlow */
    EMIT(b, 0, bit & 1u, b->ttrail);
    return t;
}

/* One byte: 8 bits + ACK slot (ack!=0 -> SDA low during the 9th clock).
 * glitch_bit: emit a glitch during that bit index (0..7, MSB-first) or <0.
 * Returns the MSB bit's SCL rising edge timestamp. */
static uint64_t wf_byte(pb_t *b, uint8_t v, uint8_t ack, int glitch_bit)
{
    uint64_t t_msb = 0;
    int i;
    for (i = 7; i >= 0; i--) {
        int g = (glitch_bit >= 0 && (7 - i) == glitch_bit);
        uint64_t t = wf_bit(b, (v >> i) & 1u, g);
        if (i == 7) {
            t_msb = t;
        }
    }
    EMIT(b, 0, ack ? 0 : 1, b->tsetup);
    EMIT(b, 1, ack ? 0 : 1, b->thigh);
    EMIT(b, 0, ack ? 0 : 1, b->ttrail);
    return t_msb;
}

/* ---------------- frame builders ---------------- */

static void ev_addr7(pb_t *b, uint64_t t, uint8_t byte, uint8_t ack)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = t;
    ev.type = I2CDEC_EV_ADDR;
    ev.addr = byte >> 1;
    ev.rw = byte & 1u;
    ev.ack = ack;
    ev.data = byte;
    add_ev(b, &ev);
}

static void ev_addr10(pb_t *b, uint64_t t, uint8_t first, uint16_t addr,
                      uint8_t rw, uint8_t ack, uint16_t flags)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = t;
    ev.type = I2CDEC_EV_ADDR;
    ev.addr = addr;
    ev.rw = rw;
    ev.ack = ack;
    ev.data = first;
    ev.flags = flags;
    add_ev(b, &ev);
}

static void ev_data(pb_t *b, uint64_t t, uint8_t v, uint8_t ack)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = t;
    ev.type = I2CDEC_EV_DATA;
    ev.data = v;
    ev.ack = ack;
    add_ev(b, &ev);
}

static uint8_t frame_data_byte(uint32_t frame, uint32_t j)
{
    if (j == 0) {
        return (uint8_t)((frame >> 8) & 0xFF);
    }
    if (j == 1) {
        return (uint8_t)(frame & 0xFF);
    }
    return (uint8_t)(j - 2);
}

static void build_frame(pb_t *b, uint32_t f)
{
    const pat_config_t *c = b->cfg;
    uint64_t t;
    int glit = c->glitch_every && (f % c->glitch_every == 0);
    int gbit = glit ? 3 : -1; /* middle of SCL-high of bit 3, first data byte */

    /* idle gap before every frame (bus rests high between frames) */
    EMIT_IDLE(b, (uint64_t)c->idle_ns * c->sys_hz / 1000000000ULL + 1);

    if (c->nack_every && (f % c->nack_every == 0)) {
        /* address NACK -> STOP */
        t = wf_start(b);
        ev_simple(b, I2CDEC_EV_START, t);
        t = wf_byte(b, 0xA0, 0, -1);
        ev_addr7(b, t, 0xA0, 0);
        t = wf_stop(b);
        ev_simple(b, I2CDEC_EV_STOP, t);
        return;
    }

    if (c->restart_every && (f % c->restart_every == 0)) {
        /* write 1 byte -> RESTART -> read N bytes (last NACKed) */
        uint32_t j;
        t = wf_start(b);
        ev_simple(b, I2CDEC_EV_START, t);
        t = wf_byte(b, 0xA0, 1, -1);
        ev_addr7(b, t, 0xA0, 1);
        t = wf_byte(b, frame_data_byte(f, 0), 1, gbit);
        ev_data(b, t, frame_data_byte(f, 0), 1);
        t = wf_restart(b);
        ev_simple(b, I2CDEC_EV_RESTART, t);
        t = wf_byte(b, 0xA1, 1, -1);
        ev_addr7(b, t, 0xA1, 1);
        for (j = 0; j < c->data_bytes; j++) {
            uint8_t v = frame_data_byte(f, j);
            uint8_t ack = (j + 1 == c->data_bytes) ? 0 : 1;
            t = wf_byte(b, v, ack, -1);
            ev_data(b, t, v, ack);
        }
        t = wf_stop(b);
        ev_simple(b, I2CDEC_EV_STOP, t);
        return;
    }

    if (c->addr10_every && (f % c->addr10_every == 0)) {
        /* 10-bit write to 0x2A5: header 0xF4 + low byte 0xA5 */
        uint32_t j;
        uint64_t th;
        t = wf_start(b);
        ev_simple(b, I2CDEC_EV_START, t);
        th = wf_byte(b, 0xF4, 1, -1);
        (void)wf_byte(b, 0xA5, 1, -1);
        ev_addr10(b, th, 0xF4, 0x2A5, 0, 1, I2CDEC_F_ADDR10);
        for (j = 0; j < c->data_bytes; j++) {
            uint8_t v = frame_data_byte(f, j);
            int g = (j == 0) ? gbit : -1;
            t = wf_byte(b, v, 1, g);
            ev_data(b, t, v, 1);
        }
        t = wf_stop(b);
        ev_simple(b, I2CDEC_EV_STOP, t);
        return;
    }

    /* plain frame: 0x50 W + N data bytes, all ACKed */
    {
        uint32_t j;
        t = wf_start(b);
        ev_simple(b, I2CDEC_EV_START, t);
        t = wf_byte(b, 0xA0, 1, -1);
        ev_addr7(b, t, 0xA0, 1);
        for (j = 0; j < c->data_bytes; j++) {
            uint8_t v = frame_data_byte(f, j);
            int g = (j == 0) ? gbit : -1;
            t = wf_byte(b, v, 1, g);
            ev_data(b, t, v, 1);
        }
        t = wf_stop(b);
        ev_simple(b, I2CDEC_EV_STOP, t);
    }
}

/* ---------------- public API ---------------- */

void pat_default_config(uint32_t sys_hz, pat_config_t *cfg)
{
    memset(cfg, 0, sizeof(*cfg));
    cfg->sys_hz = sys_hz;
    cfg->scl_hz = 100000;
    cfg->idle_ns = 5000;
    cfg->frames = 8;
    cfg->data_bytes = 4;
    cfg->glitch_line = PAT_LINE_NONE;
    cfg->offset_ns = PAT_CH1_OFF;
}

int pat_build(const pat_config_t *cfg,
              uint32_t *segs, size_t seg_cap, size_t *seg_n,
              i2cdec_event_t *evs, size_t ev_cap, size_t *ev_n,
              pat_info_t *info, char *err, size_t err_len)
{
    pb_t b;
    uint32_t f;
    const char *msg;

    if (err && err_len) {
        err[0] = '\0';
    }
    if (!cfg || !segs || !seg_n || !evs || !ev_n) {
        return -1;
    }
    if (cfg->sys_hz == 0 || cfg->scl_hz == 0 || cfg->frames == 0) {
        msg = "sys_hz/scl_hz/frames must be non-zero";
        goto einval;
    }
    if (cfg->data_bytes > 32) {
        msg = "data_bytes > 32";
        goto einval;
    }
    if (cfg->glitch_line != PAT_LINE_NONE && cfg->glitch_every == 0) {
        msg = "glitch_line set but glitch_every == 0";
        goto einval;
    }
    if (cfg->glitch_line == PAT_LINE_NONE && cfg->glitch_every != 0) {
        msg = "glitch_every set but glitch_line is none";
        goto einval;
    }

    memset(&b, 0, sizeof(b));
    b.cfg = cfg;
    b.segs = segs;
    b.seg_cap = seg_cap;
    b.evs = evs;
    b.ev_cap = ev_cap;
    b.err_s = err;
    b.err_len = err_len;
    b.scl = 1;
    b.sda = 1;
    b.min_hold = PAT_SEG_MAX_HOLD;

    /* SCL timing: period cycles per bit, split into tsetup (data setup) +
     * thigh (SCL high) + ttrail (rest of the low phase) so the low phase
     * totals tlow and rising-to-rising intervals equal period exactly. */
    b.period = (uint32_t)(((uint64_t)cfg->sys_hz + cfg->scl_hz / 2) /
                          cfg->scl_hz);
    b.thigh = b.period / 2;
    b.tlow = b.period - b.thigh;
    b.tsetup = b.tlow / 2;
    b.ttrail = b.tlow - b.tsetup;
    if (b.tsetup < PAT_SEG_MIN_HOLD || b.ttrail < PAT_SEG_MIN_HOLD ||
        b.thigh < PAT_SEG_MIN_HOLD) {
        msg = "scl_hz too high for sys_hz (segments < 3 cycles)";
        goto einval;
    }

    if (cfg->glitch_every != 0) {
        /* glitch must fit inside the SCL-high time with >= PAT_SEG_MIN_HOLD
         * cycles left before and after the pulse */
        if (cfg->glitch_cycles < PAT_SEG_MIN_HOLD) {
            msg = "glitch_cycles < 3 (PIO minimum segment)";
            goto einval;
        }
        if (b.thigh < cfg->glitch_cycles + 2 * PAT_SEG_MIN_HOLD) {
            msg = "glitch_cycles too wide for the SCL high time";
            goto einval;
        }
    }

    if (cfg->offset_ns < PAT_CH1_OFF) {
        msg = "offset_ns must be >= 0 (or -1 to disable CH1)";
        goto einval;
    }
    if (cfg->offset_ns > PAT_CH1_OFF) {
        uint64_t off_cyc = ((uint64_t)(uint32_t)cfg->offset_ns * cfg->sys_hz +
                            500000000ULL) / 1000000000ULL;
        if (off_cyc > 0 && off_cyc < PAT_SEG_MIN_HOLD) {
            msg = "offset_ns resolves to 1..2 cycles (< PIO minimum)";
            goto einval;
        }
    }

    for (f = 0; f < cfg->frames && !b.err; f++) {
        build_frame(&b, f);
    }

    if (b.err) {
        return b.err;
    }

    *seg_n = b.seg_n;
    *ev_n = b.ev_n;
    if (info) {
        info->loop_cycles = b.t;
        info->busy_cycles = b.busy;
        info->min_hold = b.min_hold;
        info->period_cycles = b.period;
        info->resolution_ns2x = 1;
    }
    return 0;

einval:
    if (err && err_len) {
        snprintf(err, err_len, "%s", msg);
    }
    return -1;
}
