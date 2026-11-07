#ifndef SIGGEN_CDC_BUFFER_H
#define SIGGEN_CDC_BUFFER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SIGGEN_CDC_RX_CAP 1024U
#define SIGGEN_CDC_TX_CAP 1024U

typedef struct {
    uint8_t rx[SIGGEN_CDC_RX_CAP];
    size_t rx_head;
    size_t rx_tail;
    size_t rx_count;
    uint8_t tx[SIGGEN_CDC_TX_CAP];
    uint32_t tx_length;
    bool tx_busy;
    bool connected;
    bool rx_overflow_pending;
    uint32_t rxoverflow;
    uint32_t txreject;
    bool session_lost;
    bool discard_rx;
    bool discard_lf;
} siggen_cdc_buffer_t;

void siggen_cdc_buffer_init(siggen_cdc_buffer_t *buffer);
void siggen_cdc_buffer_connected(siggen_cdc_buffer_t *buffer, bool connected);
bool siggen_cdc_buffer_take_session_lost(siggen_cdc_buffer_t *buffer);
void siggen_cdc_buffer_rx(siggen_cdc_buffer_t *buffer, const uint8_t *data, size_t length);
int siggen_cdc_buffer_getchar(siggen_cdc_buffer_t *buffer);
bool siggen_cdc_buffer_can_write(const siggen_cdc_buffer_t *buffer);
bool siggen_cdc_buffer_write(siggen_cdc_buffer_t *buffer, const uint8_t *data, size_t length);
const uint8_t *siggen_cdc_buffer_tx_data(const siggen_cdc_buffer_t *buffer);
uint32_t siggen_cdc_buffer_tx_length(const siggen_cdc_buffer_t *buffer);
void siggen_cdc_buffer_tx_done(siggen_cdc_buffer_t *buffer);

#endif
