/*
 * refgen: deterministic synthetic I2C waveform generator (Pico / Pico 2).
 * CH0 SCL=GP2 SDA=GP3, CH1 SCL=GP4 SDA=GP5, SYNC=GP6 (~1us pulse at loop
 * start). Commands over USB CDC serial.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "pico/stdlib.h"
#include "pico/stdio_usb.h"
#include "hardware/pio.h"
#include "hardware/dma.h"
#include "hardware/clocks.h"
#include "hardware/gpio.h"
#include "refgen.pio.h"
#include "pattern.h"

#define CH0_BASE 2 /* GP2=SCL GP3=SDA */
#define CH1_BASE 4 /* GP4=SCL GP5=SDA */
#define SYNC_PIN 6
#define NCH 2

#define EV_CAP 4096

static uint32_t g_segs[PAT_MAX_SEGS];
static i2cdec_event_t g_evs[EV_CAP];
static size_t g_seg_n, g_ev_n;
static pat_config_t g_cfg;
static uint32_t g_sys_hz;
static int g_built, g_running;

static PIO g_pio = pio0;
static int g_sm[NCH], g_dma_d[NCH], g_dma_c[NCH];
static uint g_prog_off;

/* CH0 data channel finishes one loop -> pulse SYNC for scope trigger */
static void dma_irq_handler(void)
{
    dma_hw->ints0 = 1u << g_dma_d[0];
    gpio_put(SYNC_PIN, 1);
    for (volatile int i = 0; i < 60; i++) {
        tight_loop_contents();
    }
    gpio_put(SYNC_PIN, 0);
}

/* claim all hardware once: SMs, program offset, DMA channels, SYNC IRQ */
static void hw_init(void)
{
    int i;
    g_sm[0] = pio_claim_unused_sm(g_pio, true);
    g_sm[1] = pio_claim_unused_sm(g_pio, true);
    g_prog_off = pio_add_program(g_pio, &refgen_program);
    for (i = 0; i < NCH; i++) {
        g_dma_d[i] = dma_claim_unused_channel(true);
        g_dma_c[i] = dma_claim_unused_channel(true);
    }
    irq_set_exclusive_handler(DMA_IRQ_0, dma_irq_handler);
    dma_channel_set_irq0_enabled(g_dma_d[0], true);
    irq_set_enabled(DMA_IRQ_0, true);

    /* SM config + pins high: idle bus before the first start */
    for (i = 0; i < NCH; i++) {
        int base = i ? CH1_BASE : CH0_BASE;
        pio_gpio_init(g_pio, base);
        pio_gpio_init(g_pio, base + 1);
        /* latch the output levels high BEFORE the pins become outputs so the
         * bus never sees a low glitch on enable. The mask is absolute GPIO
         * bits (bit n = GPIOn), not an offset from the out base. */
        pio_sm_set_pins_with_mask(g_pio, g_sm[i], 0x3u << base, 0x3u << base);
        pio_sm_set_consecutive_pindirs(g_pio, g_sm[i], base, 2, true);
    }
}

/* reconfigure SMs + DMA channel pairs for the current g_segs/g_seg_n */
static void hw_arm(void)
{
    int i;
    for (i = 0; i < NCH; i++) {
        int base = i ? CH1_BASE : CH0_BASE;
        pio_sm_config c = refgen_program_get_default_config(g_prog_off);
        sm_config_set_out_pins(&c, base, 2);
        sm_config_set_out_shift(&c, true, true, 32);
        sm_config_set_fifo_join(&c, PIO_FIFO_JOIN_TX);
        sm_config_set_clkdiv(&c, 1.0f);
        pio_sm_init(g_pio, g_sm[i], g_prog_off, &c);
        pio_sm_set_pins_with_mask(g_pio, g_sm[i], 0x3u << base, 0x3u << base);
        pio_sm_clear_fifos(g_pio, g_sm[i]);

        {
            dma_channel_config dc =
                dma_channel_get_default_config(g_dma_d[i]);
            channel_config_set_transfer_data_size(&dc, DMA_SIZE_32);
            channel_config_set_read_increment(&dc, true);
            channel_config_set_write_increment(&dc, false);
            channel_config_set_dreq(&dc, pio_get_dreq(g_pio, g_sm[i], true));
            channel_config_set_chain_to(&dc, g_dma_c[i]);
            dma_channel_configure(g_dma_d[i], &dc, &g_pio->txf[g_sm[i]],
                                  g_segs, g_seg_n, false);
        }
        {
            dma_channel_config cc =
                dma_channel_get_default_config(g_dma_c[i]);
            channel_config_set_transfer_data_size(&cc, DMA_SIZE_32);
            channel_config_set_read_increment(&cc, false);
            channel_config_set_write_increment(&cc, false);
            dma_channel_configure(g_dma_c[i], &cc,
                &dma_channel_hw_addr(g_dma_d[i])->al3_read_addr_trig,
                &g_segs[0], 1, false);
        }
    }
}

static void hw_stop(void)
{
    int i;
    if (!g_running) {
        return;
    }
    pio_set_sm_mask_enabled(g_pio, 0x3, false);
    for (i = 0; i < NCH; i++) {
        int base = i ? CH1_BASE : CH0_BASE;
        dma_channel_abort(g_dma_d[i]);
        dma_channel_abort(g_dma_c[i]);
        pio_sm_clear_fifos(g_pio, g_sm[i]);
        /* lines high again: idle bus, no glitch on disable */
        pio_sm_set_pins_with_mask(g_pio, g_sm[i], 0x3u << base, 0x3u << base);
    }
    g_running = 0;
}

static int hw_start(void)
{
    int i;
    int64_t off_cyc;
    if (!g_built) {
        return -1;
    }
    hw_stop();
    hw_arm();

    off_cyc = ((int64_t)g_cfg.offset_ns * g_sys_hz + 500000000) / 1000000000;
    if (off_cyc > 0) {
        /* CH1 executes one idle segment first = delayed copy of CH0.
         * pat_build already rejected 0 < off_cyc < PAT_SEG_MIN_HOLD. */
        pio_sm_put(g_pio, g_sm[1],
                   PAT_SEG_WORD(1, 1, (uint32_t)off_cyc));
    }
    for (i = 0; i < NCH; i++) {
        dma_channel_start(g_dma_d[i]);
    }
    if (g_cfg.offset_ns == PAT_CH1_OFF) {
        pio_enable_sm_mask_in_sync(g_pio, 0x1);
    } else {
        pio_enable_sm_mask_in_sync(g_pio, 0x3);
    }
    g_running = 1;
    return 0;
}

/* ---- serial command interface ---- */

static void do_build(char *err, size_t el)
{
    pat_info_t info;
    int rc = pat_build(&g_cfg, g_segs, PAT_MAX_SEGS, &g_seg_n,
                       g_evs, EV_CAP, &g_ev_n, &info, err, el);
    if (rc == 0) {
        g_built = 1;
        printf("OK period_ns=%llu load=%u.%01u%% segs=%zu min_seg_cycles=%lu "
               "res_ns=%.3f scl_hz_actual=%lu\n",
               info.loop_cycles * 1000ULL / (g_sys_hz / 1000000ULL),
               (unsigned)(info.busy_cycles * 100 /
                          (info.loop_cycles ? info.loop_cycles : 1)),
               (unsigned)((info.busy_cycles * 1000 /
                           (info.loop_cycles ? info.loop_cycles : 1)) % 10),
               g_seg_n, (unsigned long)info.min_hold,
               1000000000.0 / g_sys_hz,
               (unsigned long)(g_sys_hz / info.period_cycles));
    }
}

static void do_cmd(char *line)
{
    char err[128];
    char cmd[32];
    if (sscanf(line, "%31s", cmd) != 1) {
        return;
    }
    if (!strcmp(cmd, "set")) {
        char key[32];
        if (sscanf(line + 3, "%31s", key) != 1) {
            printf("ERR usage: set <key> <value>\n");
            return;
        }
        {
            const char *v = strchr(line + 3 + strlen(key) + 1, ' ');
            long val = v ? strtol(v + 1, NULL, 0) : 0;
            pat_config_t *c = &g_cfg;
            if (!strcmp(key, "freq_hz")) c->scl_hz = val;
            else if (!strcmp(key, "idle_ns")) c->idle_ns = val;
            else if (!strcmp(key, "frames")) c->frames = val;
            else if (!strcmp(key, "bytes")) c->data_bytes = val;
            else if (!strcmp(key, "nack_every")) c->nack_every = val;
            else if (!strcmp(key, "restart_every")) c->restart_every = val;
            else if (!strcmp(key, "addr10_every")) c->addr10_every = val;
            else if (!strcmp(key, "glitch_cycles")) c->glitch_cycles = val;
            else if (!strcmp(key, "glitch_every")) c->glitch_every = val;
            else if (!strcmp(key, "offset_ns")) c->offset_ns = val;
            else if (!strcmp(key, "glitch_line")) {
                if (v && !strncmp(v + 1, "scl", 3)) c->glitch_line = PAT_LINE_SCL;
                else if (v && !strncmp(v + 1, "sda", 3)) c->glitch_line = PAT_LINE_SDA;
                else c->glitch_line = PAT_LINE_NONE;
            } else {
                printf("ERR unknown key %s\n", key);
                return;
            }
        }
        g_built = 0;
        printf("OK\n");
    } else if (!strcmp(cmd, "build")) {
        err[0] = 0;
        do_build(err, sizeof(err));
        if (err[0]) {
            printf("ERR %s\n", err);
        }
    } else if (!strcmp(cmd, "start")) {
        if (hw_start() == 0) {
            printf("OK running\n");
        } else {
            printf("ERR not built\n");
        }
    } else if (!strcmp(cmd, "stop")) {
        hw_stop();
        printf("OK stopped\n");
    } else if (!strcmp(cmd, "status")) {
        printf("OK running=%d built=%d sys_hz=%lu freq=%lu idle_ns=%lu frames=%lu bytes=%lu "
               "nack=%lu restart=%lu addr10=%lu glitch_line=%d glitch_cyc=%lu glitch_every=%lu offset_ns=%ld segs=%zu evs=%zu\n",
               g_running, g_built, (unsigned long)g_sys_hz,
               (unsigned long)g_cfg.scl_hz, (unsigned long)g_cfg.idle_ns,
               (unsigned long)g_cfg.frames, (unsigned long)g_cfg.data_bytes,
               (unsigned long)g_cfg.nack_every, (unsigned long)g_cfg.restart_every,
               (unsigned long)g_cfg.addr10_every, g_cfg.glitch_line,
               (unsigned long)g_cfg.glitch_cycles, (unsigned long)g_cfg.glitch_every,
               (long)g_cfg.offset_ns, g_seg_n, g_ev_n);
    } else if (!strcmp(cmd, "dump")) {
        size_t i;
        for (i = 0; i < g_ev_n; i++) {
            i2cdec_event_t *e = &g_evs[i];
            printf("%llu,%u,%u,%u,0x%02X,%u,0x%X\n",
                   e->t * 1000ULL / (g_sys_hz / 1000000ULL),
                   e->type, e->addr, e->rw, e->data, e->ack, e->flags);
        }
        printf("OK %zu events (expected events assume glitches are removed "
               "by the analyzer filter)\n", g_ev_n);
    } else {
        printf("ERR unknown command\n");
    }
}

int main(void)
{
    static char line[128];
    static size_t linelen;
    int ch;

    g_sys_hz = clock_get_hz(clk_sys);
    pat_default_config(g_sys_hz, &g_cfg);

    stdio_usb_init();
    gpio_init(SYNC_PIN);
    gpio_set_dir(SYNC_PIN, true);
    gpio_put(SYNC_PIN, 0);

    hw_init();

    printf("refgen ready\n");

    for (;;) {
        ch = getchar_timeout_us(1000);
        if (ch == PICO_ERROR_TIMEOUT) {
            continue;
        }
        if (ch == '\r' || ch == '\n') {
            if (linelen) {
                line[linelen] = 0;
                do_cmd(line);
                linelen = 0;
            }
        } else if (linelen < sizeof(line) - 1) {
            line[linelen++] = (char)ch;
        }
    }
    return 0;
}
