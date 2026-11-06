#ifndef BRIDGE_PROTOCOL_H
#define BRIDGE_PROTOCOL_H
#include "i2c_port.h"
typedef struct {
    uint8_t *response;
    uint16_t response_len;
    uint8_t command, pending, count;
    bridge_transfer_t transfer[2];
    bridge_result_t result[2];
    uint8_t data[2][256];
} bridge_protocol_t;
/* Portable parser shared by firmware and native contract tests. Caller owns
 * request/response until completion; endpoint capacity must be >=512 bytes. */
void bridge_protocol_start(bridge_protocol_t *, const uint8_t *request, uint32_t len,
                           uint16_t endpoint_mps, uint8_t *response);
bool bridge_protocol_poll(bridge_protocol_t *);
#endif
