#ifndef BRIDGE_I2C_DEADLINE_H
#define BRIDGE_I2C_DEADLINE_H
#include <stdbool.h>
#include <stdint.h>
typedef struct { uint64_t started, low_since; bool low; } bridge_deadline_t;
/* 0 pending, 1 total deadline, 2 SCL-low deadline. Byte progress proves
 * intervening clocks and prevents aliasing of short high pulses. */
static inline unsigned bridge_deadline_step(bridge_deadline_t *d, uint64_t now,
    bool scl_high, bool byte_progress, uint32_t total_us, uint32_t stretch_us, uint32_t low_us)
{
    if (now - d->started >= total_us) return 1;
    if (scl_high || byte_progress) d->low = false;
    if (!scl_high) {
        if (!d->low) { d->low_since = now; d->low = true; }
        if (now - d->low_since >= (uint64_t)stretch_us + low_us) return 2;
    }
    return 0;
}
#endif
