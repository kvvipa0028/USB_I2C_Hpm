/*
 * Waveform generator producing timestamped SCL/SDA edge arrays for the
 * i2c_decoder unit tests. Host-side test helper.
 */
#ifndef WAVEGEN_H
#define WAVEGEN_H

#include <stddef.h>
#include <stdint.h>

#include "i2c_decoder.h"

typedef struct {
    i2cdec_edge_t *edges;
    size_t         cap;
    size_t         n;
    uint64_t       t;
    uint8_t        lvl[2];     /* current level of each line */
    uint32_t       tsetup;     /* SDA setup time before SCL rises */
    uint32_t       tlow;       /* SCL low half-period */
    uint32_t       thigh;      /* SCL high half-period */
} wavegen_t;

void wg_init(wavegen_t *w, i2cdec_edge_t *buf, size_t cap, uint64_t t0);
void wg_set_timing(wavegen_t *w, uint32_t tsetup, uint32_t tlow, uint32_t thigh);

void     wg_wait(wavegen_t *w, uint64_t ticks);
void     wg_edge(wavegen_t *w, uint8_t line, uint8_t level);

/* Returns the timestamp of the condition's SDA edge. */
uint64_t wg_start(wavegen_t *w);
uint64_t wg_restart(wavegen_t *w);
uint64_t wg_stop(wavegen_t *w);

/* Emits one byte: 8 data bits MSB-first plus the ACK slot
 * (ack != 0 drives SDA low during the 9th clock).
 * Returns the timestamp of the MSB bit's SCL rising edge. */
uint64_t wg_byte(wavegen_t *w, uint8_t value, uint8_t ack);

/* Emits a single data bit (SDA change + SCL pulse).
 * Returns the SCL rising edge timestamp. */
uint64_t wg_bit(wavegen_t *w, uint8_t bit);

/* Inserts a glitch (two opposite edges `width` ticks apart starting at t)
 * into a sorted edge array. */
void wg_insert_glitch(i2cdec_edge_t *edges, size_t *n, size_t cap,
                      uint8_t line, uint64_t t, uint64_t width);

#endif /* WAVEGEN_H */
