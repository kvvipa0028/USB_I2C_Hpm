/*
 * Host unit tests for the portable I2C decoder.
 * Plain C99, tiny assert macro, no external frameworks.
 * Usage: test_i2c_decoder [case_name]   (no arg runs all)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "i2c_decoder.h"
#include "wavegen.h"

static int g_fail;

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond)) {                                                         \
            printf("FAIL %s:%d: %s\n", __func__, __LINE__, #cond);             \
            g_fail++;                                                          \
        }                                                                      \
    } while (0)

/* ---------------- event capture ---------------- */

static i2cdec_event_t *g_evs;
static size_t          g_evs_cap;
static size_t          g_nev;

static void emit_cb(void *user, const i2cdec_event_t *ev)
{
    (void)user;
    if (g_nev < g_evs_cap) {
        g_evs[g_nev++] = *ev;
    }
}

static size_t decode(const i2cdec_edge_t *edges, size_t n,
                     const i2cdec_config_t *cfg, uint64_t flush_now,
                     i2cdec_event_t *out, size_t out_cap, i2cdec_stats_t *stats)
{
    i2cdec_t d;
    g_evs = out;
    g_evs_cap = out_cap;
    g_nev = 0;
    i2cdec_init(&d, cfg, 1, 1, emit_cb, NULL);
    i2cdec_push(&d, edges, n);
    i2cdec_flush(&d, flush_now);
    if (stats) {
        i2cdec_get_stats(&d, stats);
    }
    return g_nev;
}

static i2cdec_event_t mk_ev(uint8_t type, uint64_t t)
{
    i2cdec_event_t e;
    memset(&e, 0, sizeof(e));
    e.type = type;
    e.t = t;
    return e;
}

static int ev_eq(const i2cdec_event_t *a, const i2cdec_event_t *b)
{
    return a->t == b->t && a->type == b->type && a->ack == b->ack &&
           a->rw == b->rw && a->data == b->data && a->addr == b->addr &&
           a->flags == b->flags && a->aux == b->aux;
}

static void ev_dump(const char *tag, const i2cdec_event_t *evs, size_t n)
{
    size_t i;
    printf("%s (%zu events):\n", tag, n);
    for (i = 0; i < n; i++) {
        const i2cdec_event_t *e = &evs[i];
        printf("  [%zu] t=%llu type=%u ack=%u rw=%u data=0x%02X "
               "addr=0x%03X flags=0x%X aux=%u\n",
               i, (unsigned long long)e->t, e->type, e->ack, e->rw,
               e->data, e->addr, e->flags, e->aux);
    }
}

static void expect_seq(const i2cdec_event_t *got, size_t ng,
                       const i2cdec_event_t *exp, size_t ne)
{
    size_t i, nmin = ng < ne ? ng : ne;
    for (i = 0; i < nmin; i++) {
        if (!ev_eq(&got[i], &exp[i])) {
            printf("FAIL: first difference at event %zu\n", i);
            ev_dump("expected", &exp[i], 1);
            ev_dump("got", &got[i], 1);
            if (i > 0) {
                ev_dump("exp-1", &exp[i - 1], 1);
                ev_dump("got-1", &got[i - 1], 1);
            }
            g_fail++;
            return;
        }
    }
    if (ng != ne) {
        printf("FAIL: expected %zu events, got %zu (common prefix ok)\n", ne, ng);
        ev_dump("expected tail", &exp[nmin], ne - nmin > 4 ? 4 : ne - nmin);
        g_fail++;
    }
}

#define EBUF 512
#define VBUF 64

static size_t gen_ref(i2cdec_edge_t *e, size_t cap, wavegen_t *w,
                      uint64_t ts[4]);
static void   fill_ref_exp(uint64_t ts[4], i2cdec_event_t *exp);

/* ------------------------------------------------------------------ */

static void t_write(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3, t4;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA0, 1); /* 0x50 W, ACK */
    t2 = wg_byte(&w, 0x00, 1);
    t3 = wg_byte(&w, 0xA5, 1);
    t4 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_DATA, t2);
    exp[2].data = 0x00; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_DATA, t3);
    exp[3].data = 0xA5; exp[3].ack = 1;
    exp[4] = mk_ev(I2CDEC_EV_STOP, t4);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 5);
}

static void t_read(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3, t4;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA1, 1); /* 0x50 R, ACK */
    t2 = wg_byte(&w, 0x11, 1);
    t3 = wg_byte(&w, 0x22, 0); /* NACK */
    t4 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].rw = 1; exp[1].ack = 1; exp[1].data = 0xA1;
    exp[2] = mk_ev(I2CDEC_EV_DATA, t2);
    exp[2].data = 0x11; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_DATA, t3);
    exp[3].data = 0x22; exp[3].ack = 0;
    exp[4] = mk_ev(I2CDEC_EV_STOP, t4);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 5);
}

static void t_restart(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3, t4, t5, t6;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA0, 1);
    t2 = wg_byte(&w, 0x10, 1);
    t3 = wg_restart(&w);
    t4 = wg_byte(&w, 0xA1, 1);
    t5 = wg_byte(&w, 0x33, 0);
    t6 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_DATA, t2);
    exp[2].data = 0x10; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_RESTART, t3);
    exp[4] = mk_ev(I2CDEC_EV_ADDR, t4);
    exp[4].addr = 0x50; exp[4].rw = 1; exp[4].ack = 1; exp[4].data = 0xA1;
    exp[5] = mk_ev(I2CDEC_EV_DATA, t5);
    exp[5].data = 0x33; exp[5].ack = 0;
    exp[6] = mk_ev(I2CDEC_EV_STOP, t6);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 7);
}

static void t_addr_nack(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0x58, 0); /* 0x2C W, NACK */
    t2 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x2C; exp[1].ack = 0; exp[1].data = 0x58;
    exp[2] = mk_ev(I2CDEC_EV_STOP, t2);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 3);
}

static void t_addr10_write(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3, t4;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xF6, 1); /* 11110 11 0: addr10 header, hi=3 */
    t2 = wg_byte(&w, 0x34, 1); /* low address byte */
    t3 = wg_byte(&w, 0x55, 1);
    t4 = wg_stop(&w);
    (void)t2;

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1); /* t = first byte's MSB clock */
    exp[1].addr = 0x334; exp[1].ack = 1; exp[1].data = 0xF6;
    exp[1].flags = I2CDEC_F_ADDR10;
    exp[2] = mk_ev(I2CDEC_EV_DATA, t3);
    exp[2].data = 0x55; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_STOP, t4);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 4);
}

static void t_addr10_read(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3, t4, t5, t6;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xF6, 1);
    t2 = wg_byte(&w, 0x34, 1);
    t3 = wg_restart(&w);
    t4 = wg_byte(&w, 0xF7, 1); /* 11110 11 1: 10-bit read */
    t5 = wg_byte(&w, 0x99, 0);
    t6 = wg_stop(&w);
    (void)t2;

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x334; exp[1].ack = 1; exp[1].data = 0xF6;
    exp[1].flags = I2CDEC_F_ADDR10;
    exp[2] = mk_ev(I2CDEC_EV_RESTART, t3);
    exp[3] = mk_ev(I2CDEC_EV_ADDR, t4);
    exp[3].addr = 0x334; exp[3].rw = 1; exp[3].ack = 1; exp[3].data = 0xF7;
    exp[3].flags = I2CDEC_F_ADDR10;
    exp[4] = mk_ev(I2CDEC_EV_DATA, t5);
    exp[4].data = 0x99; exp[4].ack = 0;
    exp[5] = mk_ev(I2CDEC_EV_STOP, t6);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 6);
}

static void t_addr10_partial_stop(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xF6, 1); /* ACKed 10-bit write header */
    t2 = wg_stop(&w);          /* but the low byte never arrives */

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x300; exp[1].ack = 1; exp[1].data = 0xF6;
    exp[1].flags = I2CDEC_F_ADDR10 | I2CDEC_F_ADDR10_PARTIAL;
    exp[2] = mk_ev(I2CDEC_EV_STOP, t2);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 3);
}

static void t_addr10_partial_restart(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, tr;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xF6, 1);         /* ACKed 10-bit write header */
    wg_bit(&w, 0);                     /* 3 bit clocks of the low byte */
    wg_bit(&w, 0);
    wg_bit(&w, 1);
    tr = wg_restart(&w);               /* then RESTART interrupts it */

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x300; exp[1].ack = 1; exp[1].data = 0xF6;
    exp[1].flags = I2CDEC_F_ADDR10 | I2CDEC_F_ADDR10_PARTIAL;
    exp[2] = mk_ev(I2CDEC_EV_ERROR, tr);
    exp[2].data = I2CDEC_ERR_BYTE_INTERRUPTED; exp[2].aux = 3;
    exp[3] = mk_ev(I2CDEC_EV_RESTART, tr);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 4);
}

static void t_tie_order(void)
{
    /* SDA fall and SCL fall at the same timestamp: the edge listed first
     * in the input must be committed first */
    static const uint32_t cfgs[2] = {5, 0};
    size_t k;
    for (k = 0; k < 2; k++) {
        i2cdec_config_t cfg = {cfgs[k], 0, 0};

        /* SDA listed first -> SDA falls while SCL still high = START */
        {
            i2cdec_edge_t e[2];
            i2cdec_event_t got[VBUF];
            size_t ng;
            e[0].t = 100; e[0].line = I2CDEC_SDA; e[0].level = 0;
            e[1].t = 100; e[1].line = I2CDEC_SCL; e[1].level = 0;
            ng = decode(e, 2, &cfg, 1000, got, VBUF, NULL);
            CHECK(ng == 1 && got[0].type == I2CDEC_EV_START && got[0].t == 100);
        }
        /* SCL listed first -> SDA falls while SCL low: nothing */
        {
            i2cdec_edge_t e[2];
            i2cdec_event_t got[VBUF];
            size_t ng;
            e[0].t = 100; e[0].line = I2CDEC_SCL; e[0].level = 0;
            e[1].t = 100; e[1].line = I2CDEC_SDA; e[1].level = 0;
            ng = decode(e, 2, &cfg, 1000, got, VBUF, NULL);
            CHECK(ng == 0);
        }
    }
}

static void t_sda_glitch(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8], nongl[VBUF];
    i2cdec_stats_t st;
    wavegen_t w;
    uint64_t ts[4], gt;
    i2cdec_config_t cfg = {5, 0, 1};
    size_t n, ng, i, k;
    uint64_t flush;

    n = gen_ref(e, EBUF, &w, ts);
    flush = w.t + 1000;

    /* 3-tick SDA-low pulse during SCL high of the '1' MSB bit of byte 1:
     * a false START+STOP pair if not filtered */
    gt = ts[1] + 10;
    wg_insert_glitch(e, &n, EBUF, I2CDEC_SDA, gt, 3);

    ng = decode(e, n, &cfg, flush, got, VBUF, &st);
    CHECK(st.glitches == 1);

    k = 0;
    for (i = 0; i < ng; i++) {
        if (got[i].type == I2CDEC_EV_GLITCH) {
            CHECK(got[i].t == gt);
            CHECK(got[i].aux == 3);
        } else {
            nongl[k++] = got[i];
        }
    }
    fill_ref_exp(ts, exp);
    expect_seq(nongl, k, exp, 4);

    /* negative control: filter off -> false RESTART + a STOP the clean
     * stream does not have (at the glitch's second edge) */
    {
        i2cdec_config_t cfg0 = {0, 0, 0};
        i2cdec_event_t got0[VBUF];
        size_t ng0 = decode(e, n, &cfg0, flush, got0, VBUF, NULL);
        int has_restart = 0, has_false_stop = 0;
        for (i = 0; i < ng0; i++) {
            if (got0[i].type == I2CDEC_EV_RESTART) {
                has_restart = 1;
            }
            if (got0[i].type == I2CDEC_EV_STOP && got0[i].t == gt + 3) {
                has_false_stop = 1;
            }
        }
        CHECK(has_restart);
        CHECK(has_false_stop);
    }
}

/* clean reference stream: START, 0x50 W, 0x00, STOP */
static size_t gen_ref(i2cdec_edge_t *e, size_t cap, wavegen_t *w,
                      uint64_t ts[4])
{
    wg_init(w, e, cap, 0);
    ts[0] = wg_start(w);
    ts[1] = wg_byte(w, 0xA0, 1);
    ts[2] = wg_byte(w, 0x00, 1);
    ts[3] = wg_stop(w);
    return w->n;
}

static void fill_ref_exp(uint64_t ts[4], i2cdec_event_t *exp)
{
    exp[0] = mk_ev(I2CDEC_EV_START, ts[0]);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, ts[1]);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_DATA, ts[2]);
    exp[2].data = 0x00; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_STOP, ts[3]);
}

static void t_glitch_filtered(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8], nongl[VBUF];
    i2cdec_stats_t st;
    wavegen_t w;
    uint64_t ts[4], gt;
    i2cdec_config_t cfg = {5, 0, 1};
    size_t n, ng, i, k;
    uint64_t flush;

    n = gen_ref(e, EBUF, &w, ts);
    flush = w.t + 1000;

    /* 3-tick SCL-low glitch during the SCL-high of byte 2's first bit */
    gt = ts[2] + 20;
    wg_insert_glitch(e, &n, EBUF, I2CDEC_SCL, gt, 3);

    ng = decode(e, n, &cfg, flush, got, VBUF, &st);
    CHECK(st.glitches == 1);

    /* the non-glitch events must decode exactly like the clean case */
    k = 0;
    for (i = 0; i < ng; i++) {
        if (got[i].type == I2CDEC_EV_GLITCH) {
            CHECK(got[i].t == gt);
            CHECK(got[i].aux == 3);
        } else {
            nongl[k++] = got[i];
        }
    }
    fill_ref_exp(ts, exp);
    expect_seq(nongl, k, exp, 4);

    /* negative control: same input, filter off -> decode differs */
    {
        i2cdec_config_t cfg0 = {0, 0, 0};
        i2cdec_event_t got0[VBUF];
        size_t ng0 = decode(e, n, &cfg0, flush, got0, VBUF, NULL);
        int same = (ng0 == 4);
        if (same) {
            for (i = 0; i < 4; i++) {
                if (!ev_eq(&got0[i], &exp[i])) {
                    same = 0;
                }
            }
        }
        CHECK(!same);
    }
}

static void t_glitch_boundary(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF];
    i2cdec_stats_t st;
    wavegen_t w;
    uint64_t ts[4];
    i2cdec_config_t cfg = {5, 0, 1};
    size_t n;

    n = gen_ref(e, EBUF, &w, ts);
    /* width == glitch_ticks: kept, not a glitch */
    wg_insert_glitch(e, &n, EBUF, I2CDEC_SCL, ts[2] + 20, 5);

    decode(e, n, &cfg, w.t + 1000, got, VBUF, &st);
    CHECK(st.glitches == 0);
}

static void t_byte_interrupted(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, ts;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    /* 4 complete data-bit clocks, then a STOP mid-byte */
    wg_bit(&w, 1);
    wg_bit(&w, 0);
    wg_bit(&w, 1);
    wg_bit(&w, 0);
    ts = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ERROR, ts);
    exp[1].data = I2CDEC_ERR_BYTE_INTERRUPTED; exp[1].aux = 4;
    exp[2] = mk_ev(I2CDEC_EV_STOP, ts);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 3);
}

static void t_unsynced(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    /* bus noise before the first START: SDA only moves while SCL is low,
       so no START/STOP can be formed */
    wg_edge(&w, I2CDEC_SCL, 0);
    wg_wait(&w, 30);
    wg_edge(&w, I2CDEC_SDA, 0);
    wg_edge(&w, I2CDEC_SDA, 1);
    wg_edge(&w, I2CDEC_SCL, 1);
    wg_wait(&w, 60);
    wg_edge(&w, I2CDEC_SCL, 0);
    wg_wait(&w, 60);
    wg_edge(&w, I2CDEC_SDA, 0);
    wg_wait(&w, 20);
    wg_edge(&w, I2CDEC_SCL, 1); /* SCL high + SDA low: still no condition */
    wg_wait(&w, 60);
    wg_edge(&w, I2CDEC_SCL, 0);
    wg_wait(&w, 60);
    wg_edge(&w, I2CDEC_SDA, 1);
    wg_wait(&w, 20);
    /* a real transaction */
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA0, 1);
    t2 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_STOP, t2);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 3);
    CHECK(ng >= 1 && got[0].type == I2CDEC_EV_START);
}

static void t_stuck_sda(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, tlow0;
    i2cdec_config_t cfg = {5, 1000, 0};
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA0, 1);
    t2 = wg_stop(&w);
    /* after the transaction: brief SCL low (< stuck_ticks), then SDA low */
    wg_edge(&w, I2CDEC_SCL, 0);
    wg_wait(&w, 100);
    tlow0 = w.t;
    wg_edge(&w, I2CDEC_SDA, 0); /* SDA goes low while SCL low: no condition */
    wg_wait(&w, 50);
    wg_edge(&w, I2CDEC_SCL, 1); /* SCL back high; SDA stays low */

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_STOP, t2);
    exp[3] = mk_ev(I2CDEC_EV_ERROR, tlow0);
    exp[3].data = I2CDEC_ERR_STUCK_SDA;
    exp[3].aux = 2500;

    /* flush at SDA_low_start + 2500 (> stuck_ticks=1000) */
    ng = decode(e, w.n, &cfg, tlow0 + 2500, got, VBUF, NULL);
    expect_seq(got, ng, exp, 4);

    /* a second flush must not re-emit */
    {
        i2cdec_t d;
        g_evs = got; g_evs_cap = VBUF; g_nev = 0;
        i2cdec_init(&d, &cfg, 1, 1, emit_cb, NULL);
        i2cdec_push(&d, e, w.n);
        i2cdec_flush(&d, tlow0 + 2500);
        i2cdec_flush(&d, tlow0 + 99999);
        CHECK(g_nev == 4);
    }
}

static void t_level_repeat(void)
{
    i2cdec_edge_t e[4];
    i2cdec_event_t got[VBUF];
    i2cdec_stats_t st;
    i2cdec_t d;
    i2cdec_config_t cfg = {5, 0, 0};
    size_t ng;

    /* two consecutive SCL rising edges: one lost falling edge upstream */
    e[0].t = 100; e[0].line = I2CDEC_SCL; e[0].level = 1;
    e[1].t = 200; e[1].line = I2CDEC_SCL; e[1].level = 1;

    g_evs = got; g_evs_cap = VBUF; g_nev = 0;
    i2cdec_init(&d, &cfg, 0 /* scl starts low */, 1, emit_cb, NULL);
    i2cdec_push(&d, e, 2);
    i2cdec_flush(&d, 1000);
    ng = g_nev;
    i2cdec_get_stats(&d, &st);

    CHECK(st.level_repeats == 1);
    CHECK(st.edges == 2);
    CHECK(ng == 0);
}

static void t_stretch(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t got[VBUF], exp[8];
    wavegen_t w;
    uint64_t t0, t1, t2, t3;
    i2cdec_config_t cfg = {5, 0, 0}; /* stuck detection off */
    size_t ng;

    wg_init(&w, e, EBUF, 0);
    t0 = wg_start(&w);
    t1 = wg_byte(&w, 0xA0, 1);
    wg_wait(&w, 1000000); /* 10 ms SCL-low clock stretch between bytes */
    t2 = wg_byte(&w, 0x00, 1);
    t3 = wg_stop(&w);

    exp[0] = mk_ev(I2CDEC_EV_START, t0);
    exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
    exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
    exp[2] = mk_ev(I2CDEC_EV_DATA, t2);
    exp[2].data = 0x00; exp[2].ack = 1;
    exp[3] = mk_ev(I2CDEC_EV_STOP, t3);

    ng = decode(e, w.n, &cfg, w.t + 1000, got, VBUF, NULL);
    expect_seq(got, ng, exp, 4);
}

static void t_timings(void)
{
    static const uint32_t halves[2] = {50, 500}; /* 1 MHz and 100 kHz */
    int k;
    for (k = 0; k < 2; k++) {
        i2cdec_edge_t  e[EBUF];
        i2cdec_event_t got[VBUF], exp[8];
        wavegen_t w;
        uint64_t t0, t1, t2, t3;
        i2cdec_config_t cfg = {5, 0, 0};
        size_t ng;

        wg_init(&w, e, EBUF, 0);
        wg_set_timing(&w, halves[k] / 2, halves[k], halves[k]);
        t0 = wg_start(&w);
        t1 = wg_byte(&w, 0xA0, 1);
        t2 = wg_byte(&w, 0xA5, 1);
        t3 = wg_stop(&w);

        exp[0] = mk_ev(I2CDEC_EV_START, t0);
        exp[1] = mk_ev(I2CDEC_EV_ADDR, t1);
        exp[1].addr = 0x50; exp[1].ack = 1; exp[1].data = 0xA0;
        exp[2] = mk_ev(I2CDEC_EV_DATA, t2);
        exp[2].data = 0xA5; exp[2].ack = 1;
        exp[3] = mk_ev(I2CDEC_EV_STOP, t3);

        ng = decode(e, w.n, &cfg, w.t + 10000, got, VBUF, NULL);
        expect_seq(got, ng, exp, 4);
    }
}

static void t_chunking(void)
{
    i2cdec_edge_t  e[EBUF];
    i2cdec_event_t ref[VBUF], got[VBUF];
    wavegen_t w;
    i2cdec_config_t cfg = {5, 0, 0};
    i2cdec_t d;
    size_t nref, ng, off;
    uint32_t seed = 0x12345u;

    wg_init(&w, e, EBUF, 0);
    wg_start(&w);
    wg_byte(&w, 0xA0, 1);
    wg_byte(&w, 0x12, 1);
    wg_restart(&w);
    wg_byte(&w, 0xA1, 1);
    wg_byte(&w, 0x34, 0);
    wg_stop(&w);

    nref = decode(e, w.n, &cfg, w.t + 1000, ref, VBUF, NULL);

    /* same stream, random chunk sizes 1..64 (fixed seed) */
    g_evs = got; g_evs_cap = VBUF; g_nev = 0;
    i2cdec_init(&d, &cfg, 1, 1, emit_cb, NULL);
    off = 0;
    while (off < w.n) {
        size_t c;
        seed = seed * 1664525u + 1013904223u;
        c = 1 + ((seed >> 8) % 64);
        if (off + c > w.n) {
            c = w.n - off;
        }
        i2cdec_push(&d, e + off, c);
        off += c;
    }
    i2cdec_flush(&d, w.t + 1000);
    ng = g_nev;

    expect_seq(got, ng, ref, nref);
}

/* ---------------- fuzz ---------------- */

static uint32_t frnd_state;
static uint32_t frnd(void)
{
    frnd_state = frnd_state * 1664525u + 1013904223u;
    return frnd_state >> 8;
}

typedef struct {
    i2cdec_event_t *exp;
    size_t          n;
    size_t          cap;
    uint8_t         addr10_valid;
    uint8_t         addr10_hi;
    uint8_t         addr10_low;
} fctx_t;

static void f_ev(fctx_t *c, const i2cdec_event_t *ev)
{
    if (c->n < c->cap) {
        c->exp[c->n++] = *ev;
    }
}

static int f_stop(fctx_t *c, wavegen_t *w)
{
    i2cdec_event_t ev = mk_ev(I2CDEC_EV_STOP, wg_stop(w));
    f_ev(c, &ev);
    return 0;
}

/* emit an address phase (7-bit or 10-bit) + expected events;
   returns 1 if the transaction continues, 0 if it ended */
static int f_addr(fctx_t *c, wavegen_t *w, int *rw_out)
{
    if (frnd() % 5 == 0) {
        /* 10-bit write */
        uint8_t hi = frnd() & 3u;
        uint8_t low = frnd() & 0xFFu;
        uint8_t first = 0xF0u | (uint8_t)(hi << 1);
        uint8_t ack1 = (frnd() % 10 < 9);
        uint64_t t1 = wg_byte(w, first, ack1);
        i2cdec_event_t ev;
        if (!ack1) {
            ev = mk_ev(I2CDEC_EV_ADDR, t1);
            ev.addr = (uint16_t)hi << 8;
            ev.data = first;
            ev.flags = I2CDEC_F_ADDR10 | I2CDEC_F_ADDR10_PARTIAL;
            f_ev(c, &ev);
            return f_stop(c, w);
        }
        {
            uint8_t ack2 = (frnd() % 10 < 9);
            uint64_t t_msb = t1;
            wg_byte(w, low, ack2);
            ev = mk_ev(I2CDEC_EV_ADDR, t_msb);
            ev.addr = ((uint16_t)hi << 8) | low;
            ev.ack = ack2;
            ev.data = first;
            ev.flags = I2CDEC_F_ADDR10;
            f_ev(c, &ev);
            c->addr10_valid = 1;
            c->addr10_hi = hi;
            c->addr10_low = low;
            if (!ack2) {
                return f_stop(c, w);
            }
            *rw_out = 0;
            return 1;
        }
    }
    {
        /* 7-bit: avoid reserved 0x00..0x07 and 0x78..0x7F, the latter would
         * legitimately decode as a 10-bit header */
        uint8_t a7 = (uint8_t)(0x08 + frnd() % 0x70);
        uint8_t rw = frnd() & 1u;
        uint8_t byte = (uint8_t)((a7 << 1) | rw);
        uint8_t ack = (frnd() % 10 < 9);
        uint64_t t = wg_byte(w, byte, ack);
        i2cdec_event_t ev = mk_ev(I2CDEC_EV_ADDR, t);
        ev.addr = a7;
        ev.rw = rw;
        ev.ack = ack;
        ev.data = byte;
        f_ev(c, &ev);
        if (!ack) {
            return f_stop(c, w);
        }
        *rw_out = rw;
        return 1;
    }
}

/* emit 0..8 data bytes + expected events;
   returns 1 to continue, 0 if the transaction ended on a NACK */
static int f_data(fctx_t *c, wavegen_t *w, int rw)
{
    int i, n = (int)(frnd() % 9);
    for (i = 0; i < n; i++) {
        uint8_t v = frnd() & 0xFFu;
        uint8_t ack = rw ? (i == n - 1 ? 0 : 1) : (frnd() % 10 < 9);
        uint64_t t = wg_byte(w, v, ack);
        i2cdec_event_t ev = mk_ev(I2CDEC_EV_DATA, t);
        ev.data = v;
        ev.ack = ack;
        f_ev(c, &ev);
        if (!ack) {
            return f_stop(c, w);
        }
    }
    return 1;
}

static void t_fuzz(void)
{
    enum { ECAP = 400000, VCAP = 80000 };
    i2cdec_edge_t  *edges = malloc(ECAP * sizeof(*edges));
    i2cdec_event_t *exp = malloc(VCAP * sizeof(*exp));
    i2cdec_event_t *got = malloc(VCAP * sizeof(*got));
    wavegen_t w;
    fctx_t c;
    i2cdec_config_t cfg = {5, 0, 0};
    int tx;
    size_t ng;

    CHECK(edges && exp && got);
    frnd_state = 0xC0FFEEu;
    wg_init(&w, edges, ECAP, 0);
    c.exp = exp; c.n = 0; c.cap = VCAP;
    c.addr10_valid = 0;

    for (tx = 0; tx < 1000; tx++) {
        i2cdec_event_t ev;
        int rw = 0, cont;
        /* inter-transaction idle */
        wg_wait(&w, 100 + frnd() % 500);
        ev = mk_ev(I2CDEC_EV_START, wg_start(&w));
        f_ev(&c, &ev);
        c.addr10_valid = 0;
        cont = f_addr(&c, &w, &rw);
        while (cont) {
            if (!f_data(&c, &w, rw)) {
                break;
            }
            if (frnd() % 3 == 0) {
                /* repeated start */
                ev = mk_ev(I2CDEC_EV_RESTART, wg_restart(&w));
                f_ev(&c, &ev);
                if (c.addr10_valid && (frnd() & 1u)) {
                    /* 10-bit read using the remembered low byte */
                    uint8_t first = 0xF0u | (uint8_t)(c.addr10_hi << 1) | 1u;
                    uint8_t ack = (frnd() % 10 < 9);
                    uint64_t t = wg_byte(&w, first, ack);
                    ev = mk_ev(I2CDEC_EV_ADDR, t);
                    ev.addr = ((uint16_t)c.addr10_hi << 8) | c.addr10_low;
                    ev.rw = 1;
                    ev.ack = ack;
                    ev.data = first;
                    ev.flags = I2CDEC_F_ADDR10;
                    f_ev(&c, &ev);
                    if (!ack) {
                        f_stop(&c, &w);
                        break;
                    }
                    rw = 1;
                    cont = 1;
                    continue;
                }
                cont = f_addr(&c, &w, &rw);
                if (!cont) {
                    break;
                }
                continue;
            }
            f_stop(&c, &w);
            break;
        }
    }

    ng = decode(edges, w.n, &cfg, w.t + 1000, got, VCAP, NULL);
    expect_seq(got, ng, exp, c.n);

    free(edges);
    free(exp);
    free(got);
}

/* ---------------- runner ---------------- */

typedef struct {
    const char *name;
    void (*fn)(void);
} case_t;

static const case_t cases[] = {
    {"write", t_write},
    {"read", t_read},
    {"restart", t_restart},
    {"addr_nack", t_addr_nack},
    {"addr10_write", t_addr10_write},
    {"addr10_read", t_addr10_read},
    {"addr10_partial_stop", t_addr10_partial_stop},
    {"addr10_partial_restart", t_addr10_partial_restart},
    {"tie_order", t_tie_order},
    {"sda_glitch", t_sda_glitch},
    {"glitch_filtered", t_glitch_filtered},
    {"glitch_boundary", t_glitch_boundary},
    {"byte_interrupted", t_byte_interrupted},
    {"unsynced", t_unsynced},
    {"stuck_sda", t_stuck_sda},
    {"level_repeat", t_level_repeat},
    {"stretch", t_stretch},
    {"timings", t_timings},
    {"chunking", t_chunking},
    {"fuzz", t_fuzz},
};

int main(int argc, char **argv)
{
    size_t i;
    int ran = 0;
    for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        if (argc < 2 || strcmp(argv[1], cases[i].name) == 0) {
            int before = g_fail;
            cases[i].fn();
            printf("%s %-18s\n", g_fail == before ? "PASS" : "FAIL",
                   cases[i].name);
            ran++;
        }
    }
    if (!ran) {
        printf("unknown test case: %s\n", argv[1]);
        return 2;
    }
    return g_fail ? 1 : 0;
}
