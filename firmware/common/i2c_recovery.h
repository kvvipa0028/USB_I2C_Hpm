#ifndef BRIDGE_I2C_RECOVERY_H
#define BRIDGE_I2C_RECOVERY_H
#include <stdbool.h>
#include <stdint.h>
enum { REC_WAIT_IDLE, REC_LOW, REC_HIGH, REC_STOP_LOW, REC_STOP_HIGH, REC_STOP_RELEASE, REC_DONE };
typedef struct {
    uint64_t started, changed, high_since;
    uint32_t stretch_us;
    uint8_t state, clocks, initial_lines, final_lines, reason;
    bool scl_low, sda_low, high_seen;
} bridge_recovery_t;
void bridge_recovery_begin(bridge_recovery_t *, uint64_t now, uint8_t lines, uint32_t stretch_us);
bool bridge_recovery_step(bridge_recovery_t *, uint64_t now, uint8_t lines);
#endif
