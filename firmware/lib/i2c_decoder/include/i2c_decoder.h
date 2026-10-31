/*
 * Portable I2C bus decoder fed with timestamped SCL/SDA edges.
 * Pure C99, no dynamic allocation, no global state: one instance per bus.
 */
#ifndef I2C_DECODER_H
#define I2C_DECODER_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

enum { I2CDEC_SCL = 0, I2CDEC_SDA = 1 };

typedef struct {
    uint64_t t;     /* in timer ticks (10 ns @ 100 MHz) */
    uint8_t  line;  /* I2CDEC_SCL / I2CDEC_SDA */
    uint8_t  level; /* new level after the edge */
} i2cdec_edge_t;

typedef enum {
    I2CDEC_EV_START = 1,
    I2CDEC_EV_RESTART,
    I2CDEC_EV_STOP,
    I2CDEC_EV_ADDR,
    I2CDEC_EV_DATA,
    I2CDEC_EV_GLITCH,
    I2CDEC_EV_ERROR
} i2cdec_ev_type_t;

enum { I2CDEC_F_ADDR10 = 1u << 0, I2CDEC_F_ADDR10_PARTIAL = 1u << 1 };
enum {
    I2CDEC_ERR_BYTE_INTERRUPTED = 1,
    I2CDEC_ERR_STUCK_SCL = 2,
    I2CDEC_ERR_STUCK_SDA = 3
};

typedef struct {
    uint64_t t;      /* START/RESTART/STOP: time of the SDA edge.
                        ADDR/DATA: time of the SCL rising edge of the first (MSB) bit;
                          for a 10-bit EV_ADDR this is the MSB clock of the FIRST header byte.
                        GLITCH: time of the glitch's first edge.
                        ERROR STUCK_*: time the line went low; BYTE_INTERRUPTED: time of the interrupting SDA edge.
                        Note: EV_GLITCH may be emitted ahead of earlier-timestamped protocol
                        events by less than glitch_ticks, so consumers must not assume
                        strictly monotonic event timestamps. */
    uint8_t  type;   /* i2cdec_ev_type_t */
    uint8_t  ack;    /* ADDR/DATA: 1 = ACK (SDA low at 9th SCL rising edge), 0 = NACK */
    uint8_t  rw;     /* ADDR: 0 = write, 1 = read */
    uint8_t  data;   /* DATA: byte value. ADDR: raw first address byte. ERROR: error code */
    uint16_t addr;   /* ADDR: 7-bit or 10-bit address */
    uint16_t flags;
    uint32_t aux;    /* GLITCH: pulse width in ticks.
                        ERROR BYTE_INTERRUPTED: number of bits received (incl. ACK slot, 0..8).
                        ERROR STUCK_*: low duration in ticks at detection (saturate at UINT32_MAX) */
} i2cdec_event_t;

typedef struct {
    uint32_t glitch_ticks;      /* pulses strictly narrower than this are removed; 0 = filter off */
    uint32_t stuck_ticks;       /* SCL or SDA continuously low for >= this many ticks -> one
                                   STUCK error per occurrence; 0 = off */
    uint8_t  emit_glitch_events;
} i2cdec_config_t;

typedef struct {
    uint32_t edges;
    uint32_t glitches;
    uint32_t level_repeats;
} i2cdec_stats_t;

typedef void (*i2cdec_emit_fn)(void *user, const i2cdec_event_t *ev);

typedef struct i2cdec {
    i2cdec_config_t cfg;
    i2cdec_emit_fn  emit;
    void           *user;

    i2cdec_stats_t stats;

    /* glitch filter: input-side level and at most one uncommitted edge per line */
    uint8_t in_level[2];
    uint8_t sm_level[2];
    struct {
        i2cdec_edge_t edge;
        uint32_t      seq;   /* arrival order, for timestamp-tie commit order */
        uint8_t       valid;
    } pend[2];
    uint32_t next_seq;

    /* stuck detection (committed levels) */
    uint64_t low_since[2];
    uint8_t  stuck_fired[2];

    /* protocol state */
    uint8_t  active;         /* between START and STOP */
    uint8_t  bitcnt;         /* completed SCL clocks in current byte, 0..8 */
    uint8_t  cur_byte;       /* shift register */
    uint8_t  pend_bit;       /* SDA level sampled at the last SCL rising edge */
    uint8_t  pend_bit_valid; /* a sampled bit awaits its SCL falling edge */
    uint8_t  phase;          /* 0 = next byte is an address byte, 1 = data */
    uint64_t byte_t;         /* t of the SCL rising edge that sampled the MSB bit */
    uint8_t  addr10_pending; /* 10-bit write header ACKed, awaiting low byte */
    uint8_t  addr10_valid;   /* low byte of a 10-bit address remembered this transaction */
    uint8_t  addr10_low;
    uint8_t  addr10_hi;
    uint8_t  addr10_first;   /* raw first header byte, for the ADDR event data field */
    uint64_t addr10_t;       /* MSB-bit time of the first header byte */
} i2cdec_t;

void i2cdec_init(i2cdec_t *d, const i2cdec_config_t *cfg,
                 uint8_t scl_level, uint8_t sda_level,
                 i2cdec_emit_fn emit, void *user);

/* edges of ONE channel, time-ordered (SCL and SDA merged by caller) */
void i2cdec_push(i2cdec_t *d, const i2cdec_edge_t *edges, size_t n);

/* commit pending edges with t + glitch_ticks <= now; evaluate stuck timeouts
 * at `now`. The caller guarantees that every edge pushed afterwards has
 * t >= now. */
void i2cdec_flush(i2cdec_t *d, uint64_t now);

void i2cdec_get_stats(const i2cdec_t *d, i2cdec_stats_t *out);

#ifdef __cplusplus
}
#endif

#endif /* I2C_DECODER_H */
