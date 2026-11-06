#include "i2c_timing.h"
#include "i2c_table.h"
#include "i2c_deadline.h"
#include "i2c_recovery.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <string.h>

static void timing_tests(void)
{
    const uint32_t clocks[] = { 24000000, 25123456, 80000000, 200000000 };
    const uint32_t rates[] = { 10000, 50000, 100000, 400000, 800000, 1000000 };
    bridge_i2c_timing_t t;
    assert(!bridge_i2c_timing(24000000, 9999, &t));
    assert(!bridge_i2c_timing(24000000, 1000001, &t));
    assert(!bridge_i2c_timing(0, 100000, &t));
    assert(!bridge_i2c_timing(UINT32_MAX, 10000, &t)); /* TPM/high fields cannot reach it. */
    for (uint32_t hz = 10000; hz <= 1000000; hz += 997) {
        if (bridge_i2c_timing(UINT32_MAX, hz, &t))
            assert(t.actual_hz >= 10000 && t.actual_hz <= 1000000);
    }
    for (unsigned c = 0; c < sizeof(clocks)/sizeof(clocks[0]); ++c) {
        for (unsigned r = 0; r < sizeof(rates)/sizeof(rates[0]); ++r) {
            assert(bridge_i2c_timing(clocks[c], rates[r], &t));
            assert(t.actual_hz >= 10000 && t.actual_hz <= 1000000);
            assert((uint64_t)t.actual_hz * 100 <= (uint64_t)rates[r] * 105 &&
                   (uint64_t)t.actual_hz * 100 >= (uint64_t)rates[r] * 95);
            assert(t.high <= 511 && t.high > t.spike && t.high > t.hold);
            uint64_t high = 2U + (UINT64_C(2) + t.spike + t.high) * (t.multiplier + 1U);
            uint64_t low = 2U + (UINT64_C(2) + t.spike + (uint64_t)t.high * t.ratio) * (t.multiplier + 1U);
            uint64_t min_h = rates[r] <= 100000 ? 4000 : rates[r] <= 400000 ? 600 : 260;
            uint64_t min_l = rates[r] <= 100000 ? 4700 : rates[r] <= 400000 ? 1300 : 500;
            assert(high * 1000000000 >= clocks[c] * min_h);
            assert(low * 1000000000 >= clocks[c] * min_l);
            assert(t.actual_hz == clocks[c] / (high + low));
        }
        for (uint32_t hz = 10000; hz <= 1000000; hz += 997) {
            bool ok = bridge_i2c_timing(clocks[c], hz, &t);
            if (!ok) fprintf(stderr, "no timing: clock=%" PRIu32 " hz=%" PRIu32 "\n", clocks[c], hz);
            assert(ok);
            assert(t.actual_hz >= 10000 && t.actual_hz <= 1000000);
        }
    }
}
static void resync(bridge_table_t *t, bool interleave)
{
    for (unsigned i = 0; i < 256; ++i) {
        if (interleave) bridge_table_external_write(t, (uint8_t)(255 - i), (uint8_t)i);
        bool done = bridge_table_resync_one(t);
        assert(done == (i == 255));
    }
    assert(t->mode == TABLE_MIRROR);
    for (unsigned i = 0; i < 256; ++i) assert(t->bytes[0][i] == t->bytes[1][i]);
}
static void table_tests(void)
{
    bridge_table_t t = {0}; uint8_t copy[256], patch[256]; uint64_t gen;
    for (unsigned i = 0; i < 256; ++i) { bridge_table_external_write(&t, (uint8_t)i, (uint8_t)i); patch[i] = 0xA5; }
    assert(bridge_table_snapshot_begin(&t, &gen) && gen == 256);
    for (unsigned i = 0; i < 256; ++i) {
        bridge_table_external_write(&t, (uint8_t)i, (uint8_t)(255 - i));
        bridge_table_snapshot_copy(&t, (uint16_t)i, 1, copy + i);
        assert(copy[i] == (uint8_t)i);
    }
    assert(!bridge_table_snapshot_begin(&t, &gen));
    bridge_table_resync_begin(&t); resync(&t, true);
    uint64_t before = t.generation;
    assert(!bridge_table_patch_begin(&t, 255, 2, false, 0));
    assert(!bridge_table_patch_begin(&t, 0, 0, false, 0));
    assert(!bridge_table_patch_begin(&t, 0, 256, true, before - 1));
    assert(bridge_table_patch_begin(&t, 1, 254, false, 0));
    bridge_table_patch_copy(&t, patch);
    bridge_table_external_write(&t, 0, 0x77); bridge_table_external_write(&t, 255, 0x88);
    bridge_table_external_write(&t, 128, 0x66);
    assert(bridge_table_read_byte(&t, 128) == 0x66);
    assert(bridge_table_patch_commit(&t, false, 0, &gen) && gen == before + 4);
    assert(bridge_table_read_byte(&t, 0) == 0x77 && bridge_table_read_byte(&t, 255) == 0x88);
    for (unsigned i = 1; i < 255; ++i) assert(bridge_table_read_byte(&t, (uint8_t)i) == 0xA5);
    bridge_table_external_write(&t, 128, 0x99); resync(&t, false);
    assert(bridge_table_read_byte(&t, 128) == 0x99);
    before = t.generation;
    assert(bridge_table_patch_begin(&t, 0, 256, true, before));
    bridge_table_patch_copy(&t, patch); bridge_table_external_write(&t, 200, 0x55);
    assert(!bridge_table_patch_commit(&t, true, before, &gen));
    resync(&t, false); assert(bridge_table_read_byte(&t, 200) == 0x55);
    assert(bridge_table_read_byte(&t, 0) == 0x77);
    before = t.generation;
    assert(bridge_table_patch_begin(&t, 255, 1, true, before));
    bridge_table_patch_copy(&t, patch);
    assert(bridge_table_patch_commit(&t, true, before, &gen) && gen == before + 1);
    resync(&t, false);
    assert(bridge_table_read_byte(&t, 255) == 0xA5);
    uint8_t offset = 254;
    for (unsigned i = 0; i < 4; ++i) bridge_table_external_write(&t, offset++, (uint8_t)(10 + i));
    assert(bridge_table_read_byte(&t, 254) == 10 && bridge_table_read_byte(&t, 255) == 11);
    assert(bridge_table_read_byte(&t, 0) == 12 && bridge_table_read_byte(&t, 1) == 13);
}
static void deadline_tests(void)
{
    bridge_deadline_t d = { .started = 100 };
    assert(!bridge_deadline_step(&d, 100, false, false, 10000, 1000, 50));
    assert(!bridge_deadline_step(&d, 1149, false, false, 10000, 1000, 50));
    assert(bridge_deadline_step(&d, 1150, false, false, 10000, 1000, 50) == 2);
    d = (bridge_deadline_t){ .started = 0 };
    for (unsigned now = 0; now < 10000; now += 100)
        assert(!bridge_deadline_step(&d, now, false, true, 10000, 1000, 1));
    assert(bridge_deadline_step(&d, 10000, true, true, 10000, 1000, 1) == 1);
    d = (bridge_deadline_t){ .started = 0 };
    assert(!bridge_deadline_step(&d, 0, false, false, 10000, 1000, 1));
    assert(!bridge_deadline_step(&d, 999, true, false, 10000, 1000, 1));
    assert(!bridge_deadline_step(&d, 1000, false, false, 10000, 1000, 1));
    assert(bridge_deadline_step(&d, 2001, false, false, 10000, 1000, 1) == 2);
}
static void recovery_tests(void)
{
    bridge_recovery_t r;
    bridge_recovery_begin(&r, 0, 3, 1000);
    unsigned edges = 0; bool was_low = false, done = false;
    for (uint64_t now = 0; now < 1000 && !done; ++now) {
        uint8_t lines = (r.scl_low ? 0 : 1) | (r.sda_low ? 0 : 2);
        done = bridge_recovery_step(&r, now, lines);
        if (was_low && !r.scl_low) ++edges;
        was_low = r.scl_low;
    }
    assert(done && r.reason == 0 && r.clocks == 9 && edges == 10 && r.final_lines == 3);
    assert(!r.scl_low && !r.sda_low);
    bridge_recovery_begin(&r, 0, 2, 1000);
    assert(!bridge_recovery_step(&r, 999, 2));
    assert(bridge_recovery_step(&r, 1000, 2) && r.reason == 1);
    assert(!r.scl_low && !r.sda_low);
    bridge_recovery_begin(&r, 0, 1, 1000);
    done = false;
    for (uint64_t now = 0; now < 10000 && !done; ++now)
        done = bridge_recovery_step(&r, now, r.scl_low ? 0 : 1);
    assert(done && r.reason == 2 && r.clocks == 9);
    bridge_recovery_begin(&r, 0, 3, 1000);
    assert(bridge_recovery_step(&r, 1500000, 3) && r.reason == 3);
    /* A delayed release still receives a full measured high phase. */
    bridge_recovery_begin(&r, 0, 3, 1000);
    assert(!bridge_recovery_step(&r, 0, 3));
    assert(!bridge_recovery_step(&r, 5, 2));
    assert(!bridge_recovery_step(&r, 900, 2));
    assert(!bridge_recovery_step(&r, 901, 3));
    assert(r.clocks == 0 && !r.scl_low);
    assert(!bridge_recovery_step(&r, 905, 3) && r.clocks == 0);
    assert(!bridge_recovery_step(&r, 906, 3) && r.clocks == 1);
}
int main(void)
{
    timing_tests(); table_tests(); deadline_tests(); recovery_tests();
    puts("PASS common: timing, table interleavings/CAS, elapsed deadlines, recovery faults");
}
