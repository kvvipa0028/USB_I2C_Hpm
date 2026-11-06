#include "i2c_timing.h"
#include <limits.h>

static uint32_t ceil_div(uint64_t n, uint32_t d)
{
    return (uint32_t)((n + d - 1U) / d);
}

/* HPM6300 SETUP/TPM equations, in source-clock cycles. No SDK timing
 * implementation is used. Only solutions within the contracted +/-5%
 * requested rate tolerance are accepted. This is a calculation, not a waveform test. */
bool bridge_i2c_timing(uint32_t clock_hz, uint32_t hz, bridge_i2c_timing_t *out)
{
    uint32_t min_high, min_low, min_setup, min_hold, mode_max_hz;
    uint32_t best_error = UINT32_MAX;
    bool found = false;
    if (!out || clock_hz < 1000000U || hz < 10000U || hz > 1000000U) return false;
    if (hz <= 100000U) {
        min_high = 4000; min_low = 4700; min_setup = 250; min_hold = 300;
        mode_max_hz = 100000;
    } else if (hz <= 400000U) {
        min_high = 600; min_low = 1300; min_setup = 100; min_hold = 300;
        mode_max_hz = 400000;
    } else {
        min_high = 260; min_low = 500; min_setup = 50; min_hold = 0;
        mode_max_hz = 1000000;
    }
    for (uint32_t mul = 1; mul <= 32; ++mul) {
        uint32_t sp = (uint32_t)((uint64_t)clock_hz * 50U / (1000000000ULL * mul));
        uint32_t base = 2U + (2U + sp) * mul;
        uint32_t su_cycles = ceil_div((uint64_t)clock_hz * min_setup, 1000000000U);
        uint32_t hd_cycles = ceil_div((uint64_t)clock_hz * min_hold, 1000000000U);
        uint32_t su = su_cycles > base ? ceil_div(su_cycles - base, mul) : 0;
        uint32_t hd = hd_cycles > base ? ceil_div(hd_cycles - base, mul) : 0;
        if (sp > 7 || su > 31 || hd > 31) continue;
        for (uint32_t ratio = 1; ratio <= 2; ++ratio) {
            uint64_t overhead = (uint64_t)2U * base * hz;
            uint32_t denominator = hz * mul * (1U + ratio);
            uint32_t near_hi = clock_hz > overhead ? (uint32_t)((clock_hz - overhead) / denominator) : 0;
            for (uint32_t candidate = 0; candidate < 2; ++candidate) {
                uint32_t hi = near_hi + candidate;
                uint32_t hc = ceil_div((uint64_t)clock_hz * min_high, 1000000000U);
                uint32_t lc = ceil_div((uint64_t)clock_hz * min_low, 1000000000U);
                uint32_t hmin = hc > base ? ceil_div(hc - base, mul) : 0;
                uint32_t lmin = lc > base ? ceil_div(lc - base, mul * ratio) : 0;
                if (hi < hmin) hi = hmin;
                if (hi < lmin) hi = lmin;
                if (hi <= sp) hi = sp + 1U;
                if (hi <= hd) hi = hd + 1U;
                if (hi > 511) continue;
                uint32_t high = base + hi * mul;
                uint32_t low = base + hi * ratio * mul;
                uint32_t actual = clock_hz / (high + low);
                if (actual < 10000 || actual > mode_max_hz) continue;
                uint32_t error = hz > actual ? hz - actual : actual - hz;
                if ((uint64_t)error * 100U > (uint64_t)hz * 5U) continue;
                if (!found || error < best_error) {
                    *out = (bridge_i2c_timing_t){ (uint8_t)(mul - 1U), (uint8_t)sp,
                        (uint8_t)su, (uint8_t)hd, (uint8_t)ratio, (uint16_t)hi,
                        actual, ceil_div((uint64_t)low * 1000000000U, clock_hz) };
                    best_error = error;
                    found = true;
                }
            }
        }
    }
    return found;
}
