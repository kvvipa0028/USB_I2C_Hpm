/*
 * Deterministic synthetic I2C waveform pattern builder.
 *
 * Pure C99, no pico-sdk dependency: builds on the host for unit tests and on
 * the Pico for firmware. Produces the 32-bit segment array consumed by the
 * PIO program plus the expected i2c_decoder event list for one loop.
 */
#ifndef REFGEN_PATTERN_H
#define REFGEN_PATTERN_H

#include <stddef.h>
#include <stdint.h>

#include "i2c_decoder.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Segment word layout (refgen.pio):
 *   bits[31:30] = (SDA, SCL) levels: bit31 -> SDA pin, bit30 -> SCL pin
 *   bits[29:0]  = x; the segment drives those levels for x + PAT_SEG_OVERHEAD
 *   PIO cycles.
 *
 * refgen.pio executes, per 32-bit pull:
 *   out x, 30        1 cycle
 *   out pins, 2      1 cycle
 *   jmp x-- (x+1 executions until fall-through)
 * so one segment = x + 3 cycles and the smallest possible hold is 3 cycles
 * (x = 0). Time resolution is 1 PIO cycle = 1 sys-clock cycle.
 */
#define PAT_SEG_OVERHEAD 3u
#define PAT_SEG_MIN_HOLD PAT_SEG_OVERHEAD
#define PAT_SEG_X_MASK   ((1u << 30) - 1u)
#define PAT_SEG_MAX_HOLD (PAT_SEG_OVERHEAD + PAT_SEG_X_MASK)

/* level encodings */
#define PAT_HI 1u
#define PAT_LO 0u
#define PAT_SEG_WORD(scl, sda, hold) \
    ((uint32_t)((((sda)&1u) << 1) | ((scl)&1u)) << 30 | \
     (((hold) - PAT_SEG_OVERHEAD) & PAT_SEG_X_MASK))
#define PAT_SEG_X(hold) (((hold) - PAT_SEG_OVERHEAD) & PAT_SEG_X_MASK)

/* max segment words the firmware stores (128 KiB RAM budget) */
#define PAT_RAM_BUDGET (128u * 1024u)
#define PAT_MAX_SEGS   (PAT_RAM_BUDGET / 4u)

enum { PAT_LINE_NONE = -1, PAT_LINE_SCL = 0, PAT_LINE_SDA = 1 };
enum { PAT_CH1_OFF = -1 };

typedef struct {
    uint32_t sys_hz;       /* clock_get_hz(clk_sys) at runtime */
    uint32_t scl_hz;       /* SCL frequency */
    uint32_t idle_ns;      /* idle gap before every frame */
    uint32_t frames;       /* frames per pattern loop */
    uint32_t data_bytes;   /* data bytes per frame, 0..32 */
    uint32_t nack_every;   /* every k-th frame: address byte NACKed + STOP; 0=off */
    uint32_t restart_every;/* every k-th frame: write 1 byte -> RESTART -> read
                              N bytes (last NACKed); 0=off */
    uint32_t addr10_every; /* every k-th frame uses 10-bit addr 0x2A5; 0=off */
    int      glitch_line;  /* PAT_LINE_SCL / PAT_LINE_SDA / PAT_LINE_NONE */
    uint32_t glitch_cycles;/* glitch pulse width in cycles */
    uint32_t glitch_every; /* every k-th frame carries the glitch; 0=off */
    int32_t  offset_ns;    /* CH1 delay vs CH0; PAT_CH1_OFF = CH1 disabled */
} pat_config_t;

typedef struct {
    uint64_t loop_cycles;  /* total cycles per pattern loop */
    uint64_t busy_cycles;  /* cycles inside frames (excl. idle gaps) */
    uint32_t min_hold;     /* shortest segment hold emitted */
    uint32_t period_cycles;/* SCL period in cycles; actual freq = sys_hz/period_cycles */
    uint32_t resolution_ns2x; /* cycles per unit: always 1, kept for API clarity */
} pat_info_t;

void pat_default_config(uint32_t sys_hz, pat_config_t *cfg);

/* Build the pattern.
 * segs/evs are caller buffers; returns:
 *   0  ok
 *  -1  invalid parameter (err[] has a message)
 *  -2  output buffers too small or segment array exceeds 128 KiB budget
 */
int pat_build(const pat_config_t *cfg,
              uint32_t *segs, size_t seg_cap, size_t *seg_n,
              i2cdec_event_t *evs, size_t ev_cap, size_t *ev_n,
              pat_info_t *info, char *err, size_t err_len);

#ifdef __cplusplus
}
#endif

#endif /* REFGEN_PATTERN_H */
