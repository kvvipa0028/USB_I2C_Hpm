#include "i2c_decoder.h"

#include <string.h>

enum { PHASE_ADDR = 0, PHASE_DATA = 1 };

static void emit(i2cdec_t *d, i2cdec_event_t *ev)
{
    d->emit(d->user, ev);
}

static void emit_byte_interrupted(i2cdec_t *d, uint64_t t)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.type = I2CDEC_EV_ERROR;
    ev.t = t;
    ev.data = I2CDEC_ERR_BYTE_INTERRUPTED;
    ev.aux = d->bitcnt;
    emit(d, &ev);
}

/* an ACKed 10-bit write header whose low byte never arrived: emit it as a
 * partial address before the condition that interrupted it */
static void emit_addr10_partial(i2cdec_t *d)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = d->addr10_t;
    ev.type = I2CDEC_EV_ADDR;
    ev.ack = 1;
    ev.data = d->addr10_first;
    ev.addr = (uint16_t)d->addr10_hi << 8;
    ev.flags = I2CDEC_F_ADDR10 | I2CDEC_F_ADDR10_PARTIAL;
    emit(d, &ev);
    d->addr10_pending = 0;
}

static void byte_done(i2cdec_t *d, uint8_t byte, uint8_t ack, uint64_t t)
{
    i2cdec_event_t ev;
    memset(&ev, 0, sizeof(ev));
    ev.t = t;

    if (d->addr10_pending) {
        /* low byte of a 10-bit write address: the event timestamp is the
         * MSB clock of the first header byte (start of the address phase) */
        ev.t = d->addr10_t;
        ev.type = I2CDEC_EV_ADDR;
        ev.rw = 0;
        ev.ack = ack;
        ev.data = d->addr10_first;
        ev.addr = ((uint16_t)d->addr10_hi << 8) | byte;
        ev.flags = I2CDEC_F_ADDR10;
        d->addr10_pending = 0;
        d->addr10_valid = 1;
        d->addr10_low = byte;
        d->phase = PHASE_DATA;
        emit(d, &ev);
        return;
    }

    if (d->phase == PHASE_ADDR) {
        if ((byte & 0xF8u) == 0xF0u) {
            uint8_t hi = (byte >> 1) & 0x3u;
            ev.type = I2CDEC_EV_ADDR;
            ev.data = byte;
            ev.ack = ack;
            if (byte & 0x1u) {
                /* 10-bit read header (after RESTART, or directly after START) */
                ev.rw = 1;
                ev.flags = I2CDEC_F_ADDR10;
                ev.addr = (uint16_t)hi << 8;
                if (d->addr10_valid) {
                    ev.addr |= d->addr10_low;
                } else {
                    ev.flags |= I2CDEC_F_ADDR10_PARTIAL;
                }
                d->phase = PHASE_DATA;
                emit(d, &ev);
            } else if (ack) {
                /* 10-bit write header: wait for the low byte */
                d->addr10_pending = 1;
                d->addr10_hi = hi;
                d->addr10_first = byte;
                d->addr10_t = t;
            } else {
                /* NACKed 10-bit write header: partial address */
                ev.rw = 0;
                ev.flags = I2CDEC_F_ADDR10 | I2CDEC_F_ADDR10_PARTIAL;
                ev.addr = (uint16_t)hi << 8;
                d->phase = PHASE_DATA;
                emit(d, &ev);
            }
        } else {
            ev.type = I2CDEC_EV_ADDR;
            ev.addr = byte >> 1;
            ev.rw = byte & 0x1u;
            ev.ack = ack;
            ev.data = byte;
            d->phase = PHASE_DATA;
            emit(d, &ev);
        }
        return;
    }

    ev.type = I2CDEC_EV_DATA;
    ev.data = byte;
    ev.ack = ack;
    emit(d, &ev);
}

/* feed a committed edge into the protocol state machine */
static void commit(i2cdec_t *d, const i2cdec_edge_t *e)
{
    uint8_t line = e->line;
    uint8_t level = e->level & 1u;

    if (level == 0) {
        d->low_since[line] = e->t;
        d->stuck_fired[line] = 0;
    }
    d->sm_level[line] = level;

    if (line == I2CDEC_SDA) {
        if (d->sm_level[I2CDEC_SCL] == 0) {
            return; /* data change while SCL low: not a START/STOP */
        }
        if (level == 0) {
            /* SDA falling while SCL high: START / repeated START */
            uint8_t was_active = d->active;
            if (d->addr10_pending) {
                emit_addr10_partial(d);
            }
            if (was_active && d->bitcnt != 0) {
                emit_byte_interrupted(d, e->t);
            }
            {
                i2cdec_event_t ev;
                memset(&ev, 0, sizeof(ev));
                ev.t = e->t;
                ev.type = was_active ? I2CDEC_EV_RESTART : I2CDEC_EV_START;
                emit(d, &ev);
            }
            d->active = 1;
            d->bitcnt = 0;
            d->cur_byte = 0;
            d->pend_bit_valid = 0;
            d->phase = PHASE_ADDR;
            d->addr10_pending = 0;
            if (!was_active) {
                d->addr10_valid = 0;
            }
        } else {
            /* SDA rising while SCL high: STOP */
            if (d->active) {
                if (d->addr10_pending) {
                    emit_addr10_partial(d);
                }
                if (d->bitcnt != 0) {
                    emit_byte_interrupted(d, e->t);
                }
                {
                    i2cdec_event_t ev;
                    memset(&ev, 0, sizeof(ev));
                    ev.t = e->t;
                    ev.type = I2CDEC_EV_STOP;
                    emit(d, &ev);
                }
                d->active = 0;
                d->bitcnt = 0;
                d->cur_byte = 0;
                d->pend_bit_valid = 0;
                d->addr10_pending = 0;
                d->addr10_valid = 0;
            }
        }
    } else {
        /* SCL edge: sample SDA on rising edges, complete the clock (and the
         * bit) on the falling edge. A START/STOP that arrives while SCL is
         * still high therefore does not count the bit it interrupted, and the
         * SCL rise that forms a normal STOP setup is not a phantom bit. */
        if (level) {
            if (d->active) {
                if (d->bitcnt == 0) {
                    d->byte_t = e->t;
                    d->cur_byte = 0;
                }
                d->pend_bit = d->sm_level[I2CDEC_SDA];
                d->pend_bit_valid = 1;
            }
        } else if (d->active && d->pend_bit_valid) {
            d->pend_bit_valid = 0;
            if (d->bitcnt < 8) {
                d->cur_byte = (uint8_t)((d->cur_byte << 1) | d->pend_bit);
                d->bitcnt++;
            } else {
                uint8_t ack = (d->pend_bit == 0) ? 1u : 0u;
                uint64_t t = d->byte_t;
                uint8_t byte = d->cur_byte;
                d->bitcnt = 0;
                d->cur_byte = 0;
                byte_done(d, byte, ack, t);
            }
        }
    }
}

/* commit pending edges that can no longer be cancelled, oldest first */
static void drain(i2cdec_t *d, uint64_t now)
{
    for (;;) {
        int best = -1;
        int l;
        for (l = 0; l < 2; l++) {
            if (d->pend[l].valid &&
                d->pend[l].edge.t + d->cfg.glitch_ticks <= now &&
                (best < 0 ||
                 d->pend[l].edge.t < d->pend[best].edge.t ||
                 (d->pend[l].edge.t == d->pend[best].edge.t &&
                  d->pend[l].seq < d->pend[best].seq))) {
                best = l;
            }
        }
        if (best < 0) {
            return;
        }
        {
            i2cdec_edge_t e = d->pend[best].edge;
            d->pend[best].valid = 0;
            commit(d, &e);
        }
    }
}

static void check_stuck(i2cdec_t *d, uint64_t now)
{
    int l;
    if (d->cfg.stuck_ticks == 0) {
        return;
    }
    for (l = 0; l < 2; l++) {
        uint64_t dur;
        if (d->sm_level[l] != 0 || d->stuck_fired[l]) {
            continue;
        }
        dur = now - d->low_since[l];
        if (dur >= d->cfg.stuck_ticks) {
            i2cdec_event_t ev;
            memset(&ev, 0, sizeof(ev));
            ev.t = d->low_since[l];
            ev.type = I2CDEC_EV_ERROR;
            ev.data = (l == I2CDEC_SCL) ? I2CDEC_ERR_STUCK_SCL : I2CDEC_ERR_STUCK_SDA;
            ev.aux = (dur > UINT32_MAX) ? UINT32_MAX : (uint32_t)dur;
            emit(d, &ev);
            d->stuck_fired[l] = 1;
        }
    }
}

void i2cdec_init(i2cdec_t *d, const i2cdec_config_t *cfg,
                 uint8_t scl_level, uint8_t sda_level,
                 i2cdec_emit_fn emit, void *user)
{
    memset(d, 0, sizeof(*d));
    if (cfg) {
        d->cfg = *cfg;
    }
    d->emit = emit;
    d->user = user;
    d->in_level[I2CDEC_SCL] = scl_level & 1u;
    d->in_level[I2CDEC_SDA] = sda_level & 1u;
    d->sm_level[I2CDEC_SCL] = scl_level & 1u;
    d->sm_level[I2CDEC_SDA] = sda_level & 1u;
}

void i2cdec_push(i2cdec_t *d, const i2cdec_edge_t *edges, size_t n)
{
    size_t i;
    for (i = 0; i < n; i++) {
        const i2cdec_edge_t *e = &edges[i];
        uint8_t line = e->line & 1u;
        uint8_t level = e->level & 1u;

        d->stats.edges++;

        if (level == d->in_level[line]) {
            /* edge lost upstream in capture */
            d->stats.level_repeats++;
            continue;
        }
        d->in_level[line] = level;

        if (d->pend[line].valid) {
            /* pend[line] is the last accepted edge on this line, so e is
             * necessarily the opposite level: a pair. Narrower than
             * glitch_ticks cancels both. */
            uint64_t width = e->t - d->pend[line].edge.t;
            if (width < d->cfg.glitch_ticks) {
                d->stats.glitches++;
                if (d->cfg.emit_glitch_events) {
                    i2cdec_event_t ev;
                    memset(&ev, 0, sizeof(ev));
                    ev.t = d->pend[line].edge.t;
                    ev.type = I2CDEC_EV_GLITCH;
                    ev.aux = (width > UINT32_MAX) ? UINT32_MAX : (uint32_t)width;
                    emit(d, &ev);
                }
                d->pend[line].valid = 0;
                continue;
            }
        }

        /* commit every pending edge that can no longer be cancelled,
         * in timestamp order across both lines */
        drain(d, e->t);

        d->pend[line].edge.t = e->t;
        d->pend[line].edge.line = line;
        d->pend[line].edge.level = level;
        d->pend[line].seq = d->next_seq++;
        d->pend[line].valid = 1;

        check_stuck(d, e->t);
    }
}

void i2cdec_flush(i2cdec_t *d, uint64_t now)
{
    drain(d, now);
    check_stuck(d, now);
}

void i2cdec_get_stats(const i2cdec_t *d, i2cdec_stats_t *out)
{
    *out = d->stats;
}
