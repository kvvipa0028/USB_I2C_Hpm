/*
 * Host tests for the refgen pattern builder: turn the generated segment
 * stream into decoder edges, run it through firmware/lib/i2c_decoder and
 * compare against the builder's expected event list.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "pattern.h"
#include "i2c_decoder.h"

static int g_fail;

#define CHECK(cond)                                                            \
    do {                                                                       \
        if (!(cond)) {                                                         \
            printf("FAIL %s:%d: %s\n", __func__, __LINE__, #cond);             \
            g_fail++;                                                          \
        }                                                                      \
    } while (0)

#define SEG_CAP (PAT_MAX_SEGS)
#define EV_CAP  8192

static uint32_t        *g_segs;
static i2cdec_event_t  *g_evs;
static i2cdec_edge_t   *g_edges;
static size_t           g_nedges;
static i2cdec_event_t  *g_got;
static size_t           g_ngot;

static void emit_cb(void *u, const i2cdec_event_t *ev)
{
    (void)u;
    g_got[g_ngot++] = *ev;
}

/* Convert segments to decoder edges. The PIO drives both pins atomically;
 * the builder never changes both lines in one segment. If that invariant is
 * ever violated, edges are emitted SCL first, SDA second, at the same
 * timestamp (documented fixed order). */
static size_t segs_to_edges(const uint32_t *segs, size_t n, i2cdec_edge_t *out)
{
    uint64_t t = 0;
    uint8_t scl = 1, sda = 1;
    size_t i, ne = 0;
    for (i = 0; i < n; i++) {
        uint8_t lv = (uint8_t)(segs[i] >> 30);
        uint32_t x = segs[i] & PAT_SEG_X_MASK;
        uint8_t nscl = lv & 1u;
        uint8_t nsda = lv >> 1;
        if (nscl != scl) {
            out[ne].t = t; out[ne].line = I2CDEC_SCL; out[ne].level = nscl; ne++;
            scl = nscl;
        }
        if (nsda != sda) {
            out[ne].t = t; out[ne].line = I2CDEC_SDA; out[ne].level = nsda; ne++;
            sda = nsda;
        }
        t += x + PAT_SEG_OVERHEAD;
    }
    return ne;
}

static int ev_eq(const i2cdec_event_t *a, const i2cdec_event_t *b)
{
    return a->t == b->t && a->type == b->type && a->ack == b->ack &&
           a->rw == b->rw && a->data == b->data && a->addr == b->addr &&
           a->flags == b->flags && a->aux == b->aux;
}

/* build + edge-convert + decode; returns mismatch count and asserts equality */
static void run_case(const char *name, pat_config_t *cfg, uint32_t glitch_ticks)
{
    size_t seg_n = 0, ev_n = 0;
    pat_info_t info;
    char err[128];
    i2cdec_t d;
    i2cdec_config_t dcfg = {glitch_ticks, 0, 0};
    i2cdec_stats_t st;
    int rc;
    size_t i;

    memset(&info, 0, sizeof(info));
    rc = pat_build(cfg, g_segs, SEG_CAP, &seg_n, g_evs, EV_CAP, &ev_n, &info,
                   err, sizeof(err));
    if (rc != 0) {
        printf("FAIL %s: pat_build rc=%d (%s)\n", name, rc, err);
        g_fail++;
        return;
    }

    /* no segment may be shorter than the PIO minimum */
    for (i = 0; i < seg_n; i++) {
        uint32_t hold = (g_segs[i] & PAT_SEG_X_MASK) + PAT_SEG_OVERHEAD;
        CHECK(hold >= PAT_SEG_MIN_HOLD);
    }
    CHECK(info.min_hold >= PAT_SEG_MIN_HOLD);
    CHECK(seg_n * 4 <= PAT_RAM_BUDGET);

    g_nedges = segs_to_edges(g_segs, seg_n, g_edges);

    g_ngot = 0;
    i2cdec_init(&d, &dcfg, 1, 1, emit_cb, NULL);
    i2cdec_push(&d, g_edges, g_nedges);
    i2cdec_flush(&d, info.loop_cycles + glitch_ticks + 1000);
    i2cdec_get_stats(&d, &st);

    if (g_ngot != ev_n) {
        printf("FAIL %s: expected %zu events, got %zu\n", name, ev_n, g_ngot);
        g_fail++;
    } else {
        for (i = 0; i < ev_n; i++) {
            if (!ev_eq(&g_got[i], &g_evs[i])) {
                printf("FAIL %s: event %zu: exp t=%llu type=%u data=0x%02X "
                       "addr=0x%X ack=%u rw=%u flags=0x%X; "
                       "got t=%llu type=%u data=0x%02X addr=0x%X ack=%u "
                       "rw=%u flags=0x%X\n",
                       name, i,
                       (unsigned long long)g_evs[i].t, g_evs[i].type,
                       g_evs[i].data, g_evs[i].addr, g_evs[i].ack,
                       g_evs[i].rw, g_evs[i].flags,
                       (unsigned long long)g_got[i].t, g_got[i].type,
                       g_got[i].data, g_got[i].addr, g_got[i].ack,
                       g_got[i].rw, g_got[i].flags);
                g_fail++;
            }
        }
    }
    if (g_fail == 0) {
        printf("PASS %-24s segs=%zu evs=%zu load=%.1f%%\n", name, seg_n, ev_n,
               100.0 * (double)info.busy_cycles / (double)info.loop_cycles);
    }
}

static void t_basic(void)
{
    static const uint32_t freqs[3] = {100000, 400000, 1000000};
    static const uint32_t sys[2] = {125000000, 150000000};
    size_t s, k;
    for (s = 0; s < 2; s++) {
        for (k = 0; k < 3; k++) {
            pat_config_t c;
            char name[64];
            pat_default_config(sys[s], &c);
            c.scl_hz = freqs[k];
            snprintf(name, sizeof(name), "basic_%ukHz_%luMHz",
                     freqs[k] / 1000, (unsigned long)(sys[s] / 1000000));
            run_case(name, &c, 5);
        }
    }
}

static void t_features(void)
{
    pat_config_t c;
    pat_default_config(125000000, &c);
    c.scl_hz = 400000;
    c.frames = 30;
    c.nack_every = 7;
    c.restart_every = 5;
    c.addr10_every = 3;
    run_case("features", &c, 5);
}

static void t_scl_glitch_filtered(void)
{
    pat_config_t c;
    pat_default_config(125000000, &c);
    c.scl_hz = 400000;
    c.frames = 8;
    c.glitch_line = PAT_LINE_SCL;
    c.glitch_cycles = 3;
    c.glitch_every = 1;
    run_case("scl_glitch_filt", &c, 5);
}

static void t_sda_glitch_filtered(void)
{
    pat_config_t c;
    pat_default_config(125000000, &c);
    c.scl_hz = 400000;
    c.frames = 8;
    c.glitch_line = PAT_LINE_SDA;
    c.glitch_cycles = 3;
    c.glitch_every = 1;
    run_case("sda_glitch_filt", &c, 5);
}

static void t_glitch_kept(void)
{
    /* glitch wider than the filter must NOT be removed */
    pat_config_t c;
    size_t seg_n = 0, ev_n = 0, i;
    pat_info_t info;
    char err[128];
    i2cdec_t d;
    i2cdec_config_t dcfg = {5, 0, 0};
    i2cdec_stats_t st;
    int differs = 0;

    pat_default_config(125000000, &c);
    c.scl_hz = 400000;
    c.frames = 4;
    c.glitch_line = PAT_LINE_SCL;
    c.glitch_cycles = 8; /* > glitch_ticks */
    c.glitch_every = 1;

    CHECK(pat_build(&c, g_segs, SEG_CAP, &seg_n, g_evs, EV_CAP, &ev_n, &info,
                    err, sizeof(err)) == 0);
    g_nedges = segs_to_edges(g_segs, seg_n, g_edges);
    g_ngot = 0;
    i2cdec_init(&d, &dcfg, 1, 1, emit_cb, NULL);
    i2cdec_push(&d, g_edges, g_nedges);
    i2cdec_flush(&d, info.loop_cycles + 10000);
    i2cdec_get_stats(&d, &st);

    CHECK(st.glitches == 0);
    if (g_ngot != ev_n) {
        differs = 1;
    } else {
        for (i = 0; i < ev_n; i++) {
            if (!ev_eq(&g_got[i], &g_evs[i])) {
                differs = 1;
            }
        }
    }
    CHECK(differs); /* extra SCL clock breaks the clean decode */
    printf("PASS glitch_kept\n");
}

/* R4: measure from the segment stream that every SCL rise->rise interval
 * inside a transaction equals `period` cycles and every SCL high == thigh */
static void t_scl_timing(uint32_t sys_hz, uint32_t scl_hz)
{
    pat_config_t c;
    size_t seg_n = 0, ev_n = 0, i;
    pat_info_t info;
    char err[128];
    uint64_t t = 0;
    uint8_t scl = 1;
    uint64_t last_rise = 0, last_fall = 0;
    uint32_t period, thigh, tlow;
    int have_rise = 0, have_fall = 0;
    size_t eq = 0, gt = 0;

    pat_default_config(sys_hz, &c);
    c.scl_hz = scl_hz;
    c.frames = 6;
    CHECK(pat_build(&c, g_segs, SEG_CAP, &seg_n, g_evs, EV_CAP, &ev_n, &info,
                    err, sizeof(err)) == 0);
    period = info.period_cycles;
    thigh = period / 2;
    tlow = period - thigh;
    CHECK(period > 0);

    for (i = 0; i < seg_n; i++) {
        uint8_t lv = (uint8_t)(g_segs[i] >> 30);
        uint32_t hold = (g_segs[i] & PAT_SEG_X_MASK) + PAT_SEG_OVERHEAD;
        uint8_t nscl = lv & 1u;
        if (nscl != scl) {
            if (nscl) {            /* SCL rise */
                if (have_rise) {
                    uint64_t d = t - last_rise;
                    if (d == period) {
                        eq++;
                    } else {
                        /* only legal at condition boundaries (>= +thigh) */
                        CHECK(d >= period + thigh);
                        gt++;
                    }
                }
                if (have_fall) {
                    CHECK(t - last_fall == tlow);
                }
                last_rise = t;
                have_rise = 1;
            } else {               /* SCL fall */
                if (have_rise) {
                    /* a bit clock's high time is exactly thigh; condition
                     * setups (RESTART/STOP/next-START) hold SCL high for
                     * >= 2*thigh */
                    uint64_t h = t - last_rise;
                    CHECK(h == thigh || h >= 2 * thigh);
                }
                last_fall = t;
                have_fall = 1;
            }
            scl = nscl;
        }
        t += hold;
    }
    CHECK(eq > 0);
    printf("PASS timing_%lukHz_%luMHz eq=%zu boundary=%zu\n",
           (unsigned long)(scl_hz / 1000),
           (unsigned long)(sys_hz / 1000000), eq, gt);
}

static void t_rejections(void)
{
    pat_config_t c;
    size_t n1 = 0, n2 = 0;
    char err[128];
    int rc;

    /* glitch_cycles < PAT_SEG_MIN_HOLD */
    pat_default_config(125000000, &c);
    c.glitch_line = PAT_LINE_SCL;
    c.glitch_cycles = 2;
    c.glitch_every = 1;
    err[0] = 0;
    rc = pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL, err,
                   sizeof(err));
    CHECK(rc == -1 && err[0]);
    printf("  reject glitch_cycles=2: %s\n", err);

    /* glitch too wide for the SCL high time (1 MHz: thigh=62 at 125 MHz,
     * w=60 leaves <3 cycles around the pulse) */
    pat_default_config(125000000, &c);
    c.scl_hz = 1000000;
    c.glitch_line = PAT_LINE_SCL;
    c.glitch_cycles = 60;
    c.glitch_every = 1;
    err[0] = 0;
    rc = pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL, err,
                   sizeof(err));
    CHECK(rc == -1 && err[0]);
    printf("  reject glitch too wide: %s\n", err);

    /* offset resolving to 1..2 cycles (8 ns at 125 MHz -> 1 cycle) */
    pat_default_config(125000000, &c);
    c.offset_ns = 8;
    err[0] = 0;
    rc = pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL, err,
                   sizeof(err));
    CHECK(rc == -1 && err[0]);
    printf("  reject offset_ns=8: %s\n", err);

    /* offset_ns < -1 is illegal (only -1 disables CH1) */
    pat_default_config(125000000, &c);
    c.offset_ns = -1000;
    err[0] = 0;
    rc = pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL, err,
                   sizeof(err));
    CHECK(rc == -1 && err[0]);
    printf("  reject offset_ns=-1000: %s\n", err);

    /* offset 0 (same-time CH1) stays legal */
    pat_default_config(125000000, &c);
    c.offset_ns = 0;
    CHECK(pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL, err,
                    sizeof(err)) == 0);

    printf("PASS rejections\n");
}

static void t_budget(void)
{
    pat_config_t c;
    size_t seg_n = 0, ev_n = 0;
    char err[128];

    pat_default_config(125000000, &c);
    c.scl_hz = 1000000;
    c.frames = 100000;
    c.data_bytes = 32;
    err[0] = 0;
    CHECK(pat_build(&c, g_segs, SEG_CAP, &seg_n, g_evs, EV_CAP, &ev_n, NULL,
                    err, sizeof(err)) == -2);
    CHECK(err[0] != 0);
    printf("PASS budget\n");
}

static void t_params(void)
{
    pat_config_t c;
    size_t n1 = 0, n2 = 0;
    char err[128];

    pat_default_config(125000000, &c);
    c.data_bytes = 33;
    CHECK(pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL,
                    err, sizeof(err)) == -1);
    pat_default_config(125000000, &c);
    c.scl_hz = 0;
    CHECK(pat_build(&c, g_segs, SEG_CAP, &n1, g_evs, EV_CAP, &n2, NULL,
                    err, sizeof(err)) == -1);
    printf("PASS params\n");
}

int main(void)
{
    g_segs = malloc(SEG_CAP * sizeof(*g_segs));
    g_evs = malloc(EV_CAP * sizeof(*g_evs));
    g_edges = malloc(4 * SEG_CAP * sizeof(*g_edges));
    g_got = malloc(EV_CAP * sizeof(*g_got));
    if (!g_segs || !g_evs || !g_edges || !g_got) {
        printf("alloc failed\n");
        return 2;
    }

    t_basic();
    t_features();
    t_scl_timing(125000000, 100000);
    t_scl_timing(125000000, 400000);
    t_scl_timing(125000000, 1000000);
    t_scl_timing(150000000, 100000);
    t_scl_timing(150000000, 400000);
    t_scl_timing(150000000, 1000000);
    t_rejections();
    t_scl_glitch_filtered();
    t_sda_glitch_filtered();
    t_glitch_kept();
    t_budget();
    t_params();

    printf("%s (%d failures)\n", g_fail ? "FAIL" : "PASS", g_fail);
    return g_fail ? 1 : 0;
}
