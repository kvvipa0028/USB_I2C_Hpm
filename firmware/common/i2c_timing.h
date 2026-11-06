#ifndef BRIDGE_I2C_TIMING_H
#define BRIDGE_I2C_TIMING_H
#include <stdbool.h>
#include <stdint.h>
typedef struct {
    uint8_t multiplier, spike, setup, hold, ratio;
    uint16_t high;
    uint32_t actual_hz, low_ns;
} bridge_i2c_timing_t;
bool bridge_i2c_timing(uint32_t clock_hz, uint32_t requested_hz,
                       bridge_i2c_timing_t *out);
#endif
