#ifndef SIGGEN_PEER_H
#define SIGGEN_PEER_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "siggen_command.h"

enum siggen_mode { SIG_MODE_IDLE, SIG_MODE_MASTER, SIG_MODE_SLAVE };
enum siggen_result { SIG_RESULT_PENDING, SIG_RESULT_OK, SIG_RESULT_ADDRESS_NACK,
    SIG_RESULT_DATA_NACK, SIG_RESULT_TIMEOUT, SIG_RESULT_ERROR, SIG_RESULT_CANCELLED,
    SIG_RESULT_NOT_RUN };
enum siggen_ack { SIG_ACK_NOW, SIG_NACK_NOW, SIG_ACK_DEFER };
typedef struct { enum siggen_result result; uint32_t raw; } siggen_result_t;
typedef struct {
    void *context;
    bool (*configure)(void *, bool slave, uint8_t address, uint32_t hz, bool address_nack);
    void (*disable)(void *);
    bool (*busy)(void *);
    void (*hold)(void *, uint8_t mask);
    void (*unhold)(void *);
    uint8_t (*lines)(void *);
    void (*ack)(void *, bool acknowledge);
    bool (*ack_valid)(void *);
    bool (*master_begin)(void *, bool read, uint8_t address);
    bool (*master_poll)(void *, bool read, siggen_result_t *result, uint8_t data[2]);
} siggen_hal_t;

typedef struct {
    siggen_hal_t hal;
    enum siggen_mode mode;
    uint8_t address, offset, tx_start, tx_next, slave_state, held_mask;
    uint32_t speed_hz, stretch_us, nack_data;
    bool address_nack, running, pending_ack, rx_first, rejected;
    uint16_t write_index;
    uint8_t master_phase, last_read[2];
    uint64_t last_round, round_started, ack_started, ack_deadline, hold_started, hold_duration_us, last_poll;
    uint32_t rounds, address_hits, rx_transactions, tx_transactions, rx_bytes, tx_bytes;
    uint32_t ack_count, nack_count, stretch_count, hold_count, cancelled_acks;
    uint64_t last_stretch_us, max_stretch_us, max_poll_gap_us;
    siggen_result_t write_result, read_result;
    uint8_t table[256];
} siggen_peer_t;

enum siggen_reply_kind { SIG_REPLY_NONE, SIG_REPLY_TEXT, SIG_REPLY_STATUS, SIG_REPLY_TABLE };
typedef struct {
    enum siggen_reply_kind kind;
    const char *text;
    siggen_peer_t snapshot;
    bool busy;
    uint8_t lines;
    uint32_t table_offset, table_length;
    uint8_t table[32];
} siggen_reply_t;

void siggen_peer_init(siggen_peer_t *p, const siggen_hal_t *hal);
/* Caller serializes with the I2C ISR; no function waits for hardware. */
void siggen_peer_poll(siggen_peer_t *p, uint64_t now);
void siggen_peer_stop(siggen_peer_t *p);
/* Prepare mutates/copies under the peer lock; format uses only that snapshot
 * and must run after unlock, with no HAL calls or live peer reads. */
void siggen_peer_prepare(siggen_peer_t *p, const siggen_command_t *cmd,
                         uint64_t now, siggen_reply_t *reply);
void siggen_peer_format(const siggen_reply_t *reply, char *out, size_t capacity);
void siggen_peer_command(siggen_peer_t *p, const siggen_command_t *cmd,
                         uint64_t now, char *out, size_t capacity);
void siggen_peer_address(siggen_peer_t *p, bool master_read);
enum siggen_ack siggen_peer_receive(siggen_peer_t *p, uint8_t value, uint64_t now);
uint8_t siggen_peer_transmit(siggen_peer_t *p);
void siggen_peer_complete(siggen_peer_t *p, uint16_t actual_tx_count);
const char *siggen_result_name(enum siggen_result result);
#endif
