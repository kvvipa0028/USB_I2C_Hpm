#include "i2c_recovery.h"
static bool finish(bridge_recovery_t *r, uint8_t lines, uint8_t reason)
{
    r->state = REC_DONE; r->reason = reason; r->final_lines = lines;
    r->scl_low = false; r->sda_low = false;
    return true;
}
void bridge_recovery_begin(bridge_recovery_t *r, uint64_t now, uint8_t lines, uint32_t stretch_us)
{
    *r = (bridge_recovery_t){ .started = now, .changed = now,
        .stretch_us = stretch_us, .initial_lines = lines };
}
bool bridge_recovery_step(bridge_recovery_t *r, uint64_t now, uint8_t lines)
{
    if (r->state == REC_DONE) return true;
    if (now - r->started >= 1500000U) return finish(r, lines, 3);
    switch (r->state) {
    case REC_WAIT_IDLE:
        if (lines & 1U) {
            r->scl_low = true; r->changed = now; r->state = REC_LOW;
        } else if (now - r->changed >= r->stretch_us) return finish(r, lines, 1);
        break;
    case REC_LOW:
        if (now - r->changed >= 5) {
            r->scl_low = false; r->high_seen = false; r->changed = now; r->state = REC_HIGH;
        }
        break;
    case REC_HIGH:
        if (!(lines & 1U)) {
            r->high_seen = false;
            if (now - r->changed >= r->stretch_us) return finish(r, lines, 1);
        } else if (!r->high_seen) {
            r->high_seen = true; r->high_since = now;
        } else if (now - r->high_since >= 5) {
            ++r->clocks; r->scl_low = true; r->changed = now;
            if (r->clocks == 9) { r->sda_low = true; r->state = REC_STOP_LOW; }
            else r->state = REC_LOW;
        }
        break;
    case REC_STOP_LOW:
        if (now - r->changed >= 5) {
            r->scl_low = false; r->high_seen = false; r->changed = now; r->state = REC_STOP_HIGH;
        }
        break;
    case REC_STOP_HIGH:
        if (!(lines & 1U)) {
            r->high_seen = false;
            if (now - r->changed >= r->stretch_us) return finish(r, lines, 1);
        } else if (!r->high_seen) {
            r->high_seen = true; r->high_since = now;
        } else if (now - r->high_since >= 5) {
            r->sda_low = false; r->changed = now; r->state = REC_STOP_RELEASE;
        }
        break;
    case REC_STOP_RELEASE:
        if (now - r->changed < 5) break;
        if (lines == 3) return finish(r, lines, 0);
        if (now - r->changed >= r->stretch_us) return finish(r, lines, (lines & 1U) ? 2 : 1);
        break;
    default: return finish(r, lines, 3);
    }
    return false;
}
