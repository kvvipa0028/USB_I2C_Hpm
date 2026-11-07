#ifndef BRIDGE_I2C_TABLE_H
#define BRIDGE_I2C_TABLE_H
#include <stdbool.h>
#include <stdint.h>
enum { TABLE_MIRROR, TABLE_FROZEN, TABLE_PATCH, TABLE_RESYNC };
typedef struct {
    volatile uint8_t bytes[2][256];
    volatile uint8_t active, mode;
    uint16_t begin, end, sync_next;
    volatile uint64_t generation;
} bridge_table_t;
/* These short operations must be called with the owning ISR excluded.
 * snapshot_copy and patch_copy deliberately run with interrupts enabled. */
void bridge_table_external_write(bridge_table_t *, uint8_t offset, uint8_t value);
uint8_t bridge_table_read_byte(const bridge_table_t *, uint8_t offset);
bool bridge_table_snapshot_begin(bridge_table_t *, uint64_t *generation);
void bridge_table_snapshot_copy(const bridge_table_t *, uint16_t offset, uint16_t len, uint8_t *out);
bool bridge_table_patch_begin(bridge_table_t *, uint16_t offset, uint16_t len, bool cas, uint64_t expected);
void bridge_table_patch_copy(bridge_table_t *, const uint8_t *data);
bool bridge_table_patch_commit(bridge_table_t *, bool cas, uint64_t expected, uint64_t *generation);
void bridge_table_resync_begin(bridge_table_t *);
bool bridge_table_resync_one(bridge_table_t *);
#endif
