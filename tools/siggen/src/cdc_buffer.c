#include "cdc_buffer.h"

static void clear_rx(siggen_cdc_buffer_t *buffer)
{
    buffer->rx_head = 0U;
    buffer->rx_tail = 0U;
    buffer->rx_count = 0U;
}

static void clear_tx(siggen_cdc_buffer_t *buffer)
{
    buffer->tx_length = 0U;
    buffer->tx_busy = false;
}

void siggen_cdc_buffer_init(siggen_cdc_buffer_t *buffer)
{
    if (buffer == NULL) return;
    buffer->rx_head = 0U;
    buffer->rx_tail = 0U;
    buffer->rx_count = 0U;
    buffer->tx_length = 0U;
    buffer->tx_busy = false;
    buffer->connected = false;
    buffer->rx_overflow_pending = false;
    buffer->rxoverflow = 0U;
    buffer->txreject = 0U;
    buffer->session_lost = false;
    buffer->discard_rx = false;
    buffer->discard_lf = false;
}

void siggen_cdc_buffer_connected(siggen_cdc_buffer_t *buffer, bool connected)
{
    if (buffer == NULL) return;
    if (!connected) {
        clear_rx(buffer);
        clear_tx(buffer);
        buffer->rx_overflow_pending = false;
        buffer->discard_rx = false;
        buffer->discard_lf = false;
        buffer->session_lost = true;
    }
    buffer->connected = connected;
}

bool siggen_cdc_buffer_take_session_lost(siggen_cdc_buffer_t *buffer)
{
    bool lost;
    if (buffer == NULL) return false;
    lost = buffer->session_lost;
    buffer->session_lost = false;
    return lost;
}

void siggen_cdc_buffer_rx(siggen_cdc_buffer_t *buffer, const uint8_t *data, size_t length)
{
    size_t i;
    if (buffer == NULL || data == NULL || !buffer->connected) return;
    for (i = 0U; i < length; ++i) {
        uint8_t byte = data[i];
        if (buffer->discard_lf) {
            buffer->discard_lf = false;
            if (byte == (uint8_t)'\n') continue;
        }
        if (buffer->discard_rx) {
            if (byte == (uint8_t)'\r') {
                buffer->discard_rx = false;
                buffer->discard_lf = true;
                continue;
            } else if (byte == (uint8_t)'\n') {
                buffer->discard_rx = false;
                continue;
            } else {
                continue;
            }
        }
        if (buffer->rx_count >= SIGGEN_CDC_RX_CAP) {
            clear_rx(buffer);
            buffer->discard_rx = true;
            buffer->rxoverflow++;
            buffer->rx_overflow_pending = true;
            buffer->session_lost = true;
            if (byte == (uint8_t)'\r' || byte == (uint8_t)'\n') {
                buffer->discard_lf = byte == (uint8_t)'\r';
                buffer->discard_rx = false;
            }
            continue;
        }
        buffer->rx[buffer->rx_head] = byte;
        buffer->rx_head = (buffer->rx_head + 1U) % SIGGEN_CDC_RX_CAP;
        buffer->rx_count++;
    }
}

int siggen_cdc_buffer_getchar(siggen_cdc_buffer_t *buffer)
{
    uint8_t byte;
    if (buffer == NULL) return -1;
    if (buffer->rx_overflow_pending) {
        buffer->rx_overflow_pending = false;
        return -2;
    }
    if (buffer->rx_count == 0U) return -1;
    byte = buffer->rx[buffer->rx_tail];
    buffer->rx_tail = (buffer->rx_tail + 1U) % SIGGEN_CDC_RX_CAP;
    buffer->rx_count--;
    return (int)byte;
}

bool siggen_cdc_buffer_can_write(const siggen_cdc_buffer_t *buffer)
{
    return buffer != NULL && buffer->connected && !buffer->session_lost && !buffer->tx_busy;
}

bool siggen_cdc_buffer_write(siggen_cdc_buffer_t *buffer, const uint8_t *data, size_t length)
{
    size_t i;
    if (!siggen_cdc_buffer_can_write(buffer) || data == NULL || length == 0U || length > SIGGEN_CDC_TX_CAP) {
        if (buffer != NULL) buffer->txreject++;
        return false;
    }
    for (i = 0U; i < length; ++i) buffer->tx[i] = data[i];
    buffer->tx_length = (uint32_t)length;
    buffer->tx_busy = true;
    return true;
}

const uint8_t *siggen_cdc_buffer_tx_data(const siggen_cdc_buffer_t *buffer)
{
    return buffer == NULL ? NULL : buffer->tx;
}

uint32_t siggen_cdc_buffer_tx_length(const siggen_cdc_buffer_t *buffer)
{
    return buffer == NULL ? 0U : buffer->tx_length;
}

void siggen_cdc_buffer_tx_done(siggen_cdc_buffer_t *buffer)
{
    if (buffer == NULL) return;
    clear_tx(buffer);
}
