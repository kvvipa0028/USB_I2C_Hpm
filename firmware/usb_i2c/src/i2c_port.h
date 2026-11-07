#ifndef BRIDGE_I2C_PORT_H
#define BRIDGE_I2C_PORT_H
#include <stdbool.h>
#include <stdint.h>
#define BRIDGE_PORT_COUNT 2U
enum { BRIDGE_OK, BRIDGE_NACK, BRIDGE_TIMEOUT, BRIDGE_BAD_REQUEST, BRIDGE_I2C_ERROR,
       BRIDGE_BUSY, BRIDGE_UNSUPPORTED, BRIDGE_CONFLICT, BRIDGE_WRONG_ROLE, BRIDGE_SERVICE_DEADLINE };
enum { BRIDGE_MASTER, BRIDGE_SLAVE };
typedef struct {
    uint8_t role, slave_address;
    uint32_t speed_hz, stretch_us, transaction_us;
} bridge_port_config_t;
typedef struct {
    bridge_port_config_t config;
    uint8_t activity;
    uint32_t calculated_hz;
    uint64_t config_generation, table_generation;
} bridge_port_state_t;
typedef struct {
    uint8_t port, address;
    uint16_t write_len, read_len;
    uint32_t speed_hz;
    const uint8_t *write_data;
    uint8_t *read_data;
    bool probe;
} bridge_transfer_t;
typedef struct { uint8_t status; uint32_t hpm_status; } bridge_result_t;
typedef struct { uint8_t clocks, initial_lines, final_lines; uint32_t reason; } bridge_recovery_result_t;
void bridge_ports_init(void);
void bridge_ports_poll(void);
void bridge_ports_abort(void);
void bridge_port_state(uint8_t port, bridge_port_state_t *out);
uint8_t bridge_port_configure(uint8_t port, const bridge_port_config_t *, bool cas, uint64_t expected);
uint8_t bridge_port_table_read(uint8_t port, uint16_t offset, uint16_t len, uint8_t *out, uint64_t *generation);
uint8_t bridge_port_table_write(uint8_t port, uint16_t offset, uint16_t len, const uint8_t *, bool cas, uint64_t expected, uint64_t *generation);
uint8_t bridge_port_transfer_validate(const bridge_transfer_t *);
void bridge_port_transfer_start(const bridge_transfer_t *);
bool bridge_port_transfer_result(uint8_t port, bridge_result_t *);
uint8_t bridge_port_recover_start(uint8_t port);
bool bridge_port_recover_result(uint8_t port, bridge_recovery_result_t *);
#endif
