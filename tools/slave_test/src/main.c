/*
 * slave_test (T1.9): external I2C master on HPM6750EVKMini that checks the
 * register-table slave in firmware/slave_proto (address 0x50). Bus speed is
 * BUS_MODE; it must match SLV_MODE in slave_proto.
 *
 * Each round: whole-table write, random read of the whole table, current
 * address reads (including after a 256-byte read), a short random read
 * followed by a current address read (prefetch must not advance the offset),
 * and a write/read across the 0xFF -> 0x00 wrap. Every read is compared with
 * a model of the table (g_exp). SEQ_ONLY=1 instead writes 0..254 at offset 0
 * once. Results are read over JTAG: g_result, g_exp.
 */

#include "board.h"
#include "hpm_i2c_drv.h"
#include "hpm_csr_drv.h"
#include "hpm_iomux.h"

#define M_I2C   BOARD_APP_I2C_BASE
#ifndef BUS_MODE
#define BUS_MODE  i2c_mode_normal   /* normal = 100k, fast = 400k, fast_plus = 1M */
#endif
#ifndef PEER_PULLUP
#define PEER_PULLUP 0   /* 1: also enable this board's internal pull-ups on SCL/SDA */
#endif
#define SLV     0x50U
#ifndef ROUNDS
#define ROUNDS  200U
#endif
#if ROUNDS < 1 || ROUNDS > 65535
#error "ROUNDS must be 1..65535 to preserve first_info's 16-bit round field"
#endif
#if PEER_PULLUP != 0 && PEER_PULLUP != 1
#error "PEER_PULLUP must be 0 or 1"
#endif
#define SEQ_ONLY 0      /* 1: only write 0..254 at offset 0 once; 0: ROUNDS-round test */

volatile struct {
    uint32_t magic;        /* 0xC0FFEE19 = run finished */
    uint32_t rounds;
    uint32_t call_fail;    /* driver calls that did not return success */
    uint32_t mismatches;   /* bytes read != model */
    uint32_t first_info;   /* first failure: round << 16 | step << 8 | index */
    uint32_t first_val;    /* status, or (got << 8 | expected) */
    uint32_t mode;         /* BUS_MODE */
    uint32_t elapsed_us;   /* wall time of all rounds */
    uint32_t peer_pullup;  /* PEER_PULLUP */
} g_result;

volatile uint8_t g_exp[256];   /* model of the slave table */

static uint8_t s_buf[256];

static void fail(uint32_t r, uint32_t step, uint32_t idx, uint32_t val)
{
    if (g_result.first_info == 0U && g_result.first_val == 0U) {
        g_result.first_info = (r << 16) | (step << 8) | idx;
        g_result.first_val = val;
    }
}

static void call(hpm_stat_t st, uint32_t r, uint32_t step)
{
    if (st != status_success) {
        g_result.call_fail++;
        fail(r, step, 0xFFU, (uint32_t)st);
    }
}

/* write n bytes of buf at offset off (n + 1 <= 256) */
static void wr(uint8_t off, const uint8_t *buf, uint32_t n, uint32_t r, uint32_t step)
{
    for (uint32_t i = 0; i < n; i++) {
        g_exp[(uint8_t)(off + i)] = buf[i];
    }
    call(i2c_master_address_write(M_I2C, SLV, &off, 1, (uint8_t *)buf, n), r, step);
}

/* compare n bytes read starting at table offset off */
static void check(uint8_t off, uint32_t n, uint32_t r, uint32_t step)
{
    for (uint32_t i = 0; i < n; i++) {
        uint8_t e = g_exp[(uint8_t)(off + i)];
        if (s_buf[i] != e) {
            g_result.mismatches++;
            fail(r, step, i, ((uint32_t)s_buf[i] << 8) | e);
        }
    }
}

__attribute__((unused)) static void rand_rd(uint8_t off, uint32_t n, uint32_t r, uint32_t step)
{
    call(i2c_master_address_read(M_I2C, SLV, &off, 1, s_buf, n), r, step);
    check(off, n, r, step);
}

__attribute__((unused)) static void cur_rd(uint8_t expect_off, uint32_t n, uint32_t r, uint32_t step)
{
    call(i2c_master_read(M_I2C, SLV, s_buf, n), r, step);
    check(expect_off, n, r, step);
}

int main(void)
{
    static uint8_t pat[256];

    i2c_config_t cfg = { .is_10bit_addressing = false, .i2c_mode = BUS_MODE };
    uint64_t t0;

    board_init();
    board_init_i2c(M_I2C);   /* clock, bus clear, pins; inits at 100k */
#if PEER_PULLUP
    /* the SDK pinmux sets open-drain only on PB11 (SCL) / PB10 (SDA) */
    HPM_IOC->PAD[IOC_PAD_PB11].PAD_CTL = IOC_PAD_PAD_CTL_OD_SET(1) | IOC_PAD_PAD_CTL_PE_SET(1) | IOC_PAD_PAD_CTL_PS_SET(1);
    HPM_IOC->PAD[IOC_PAD_PB10].PAD_CTL = IOC_PAD_PAD_CTL_OD_SET(1) | IOC_PAD_PAD_CTL_PE_SET(1) | IOC_PAD_PAD_CTL_PS_SET(1);
#endif
    if (BUS_MODE != i2c_mode_normal &&   /* board_init_i2c already set up 100k */
        i2c_init_master(M_I2C, clock_get_frequency(BOARD_APP_I2C_CLK_NAME), &cfg) != status_success) {
        while (1) {
        }
    }

    t0 = hpm_csr_get_core_cycle();
#if SEQ_ONLY
    for (uint32_t i = 0; i < 255; i++) {
        pat[i] = (uint8_t)i;
    }
    wr(0x00, pat, 255, 0, 1);          /* offset 0x00, then data 0..254 */
#else
    for (uint32_t r = 0; r < ROUNDS; r++) {
        uint8_t wrap[4];

        for (uint32_t i = 0; i < 256; i++) {
            pat[i] = (uint8_t)(i * 7U + r * 13U + 1U);
        }
        wr(0x00, pat, 255, r, 1);          /* 0x00..0xFE */
        wr(0xFF, &pat[255], 1, r, 1);      /* 0xFF; offset now wraps to 0x00 */
        rand_rd(0x00, 256, r, 2);          /* whole table; offset back at 0x00 */
        cur_rd(0x00, 4, r, 3);             /* current read after 256 bytes */
        cur_rd(0x04, 4, r, 4);
        rand_rd(0x80, 10, r, 5);           /* short read: FIFO may prefetch */
        cur_rd(0x8A, 6, r, 6);             /* must continue at 0x8A */
        for (uint32_t i = 0; i < 4; i++) {
            wrap[i] = (uint8_t)(0xA0U + r + i);
        }
        wr(0xFE, wrap, 4, r, 7);           /* 0xFE, 0xFF, 0x00, 0x01 */
        rand_rd(0xFE, 4, r, 8);
        g_result.rounds = r + 1U;
    }
#endif
    g_result.elapsed_us = (uint32_t)((hpm_csr_get_core_cycle() - t0) /
                                     (clock_get_frequency(clock_cpu0) / 1000000U));
    g_result.mode = BUS_MODE;
    g_result.peer_pullup = PEER_PULLUP;
    g_result.magic = 0xC0FFEE19;

    while (1) {
    }
    return 0;
}
