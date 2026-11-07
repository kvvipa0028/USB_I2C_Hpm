#include "i2c_table.h"
void bridge_table_external_write(bridge_table_t *t, uint8_t off, uint8_t value)
{
    t->bytes[t->active][off] = value;
    if (t->mode != TABLE_FROZEN &&
        !(t->mode == TABLE_PATCH && off >= t->begin && off < t->end))
        t->bytes[t->active ^ 1U][off] = value;
    ++t->generation;
}
uint8_t bridge_table_read_byte(const bridge_table_t *t, uint8_t off)
{
    return t->bytes[t->active][off];
}
bool bridge_table_snapshot_begin(bridge_table_t *t, uint64_t *generation)
{
    if (t->mode != TABLE_MIRROR) return false;
    t->mode = TABLE_FROZEN;
    *generation = t->generation;
    return true;
}
void bridge_table_snapshot_copy(const bridge_table_t *t, uint16_t off, uint16_t len, uint8_t *out)
{
    for (uint16_t i = 0; i < len; ++i) out[i] = t->bytes[t->active ^ 1U][off + i];
}
bool bridge_table_patch_begin(bridge_table_t *t, uint16_t off, uint16_t len, bool cas, uint64_t expected)
{
    if (t->mode != TABLE_MIRROR || !len || off >= 256 || len > 256U - off ||
        (cas && expected != t->generation)) return false;
    t->begin = off; t->end = off + len; t->mode = TABLE_PATCH;
    return true;
}
void bridge_table_patch_copy(bridge_table_t *t, const uint8_t *data)
{
    for (uint16_t i = t->begin; i < t->end; ++i)
        t->bytes[t->active ^ 1U][i] = data[i - t->begin];
}
bool bridge_table_patch_commit(bridge_table_t *t, bool cas, uint64_t expected, uint64_t *generation)
{
    /* Recheck at the linearization point: an external ACK during staging
     * must not let stale CAS overwrite its newer generation. */
    bool ok = !cas || t->generation == expected;
    if (ok) { t->active ^= 1U; ++t->generation; }
    *generation = t->generation;
    bridge_table_resync_begin(t);
    return ok;
}
void bridge_table_resync_begin(bridge_table_t *t)
{
    t->sync_next = 0; t->mode = TABLE_RESYNC;
}
bool bridge_table_resync_one(bridge_table_t *t)
{
    if (t->mode != TABLE_RESYNC) return t->mode == TABLE_MIRROR;
    uint16_t i = t->sync_next++;
    t->bytes[t->active ^ 1U][i] = t->bytes[t->active][i];
    if (t->sync_next == 256) t->mode = TABLE_MIRROR;
    return t->mode == TABLE_MIRROR;
}
