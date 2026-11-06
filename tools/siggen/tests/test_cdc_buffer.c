#include "../src/cdc_buffer.h"

#include <assert.h>

int main(void)
{
    siggen_cdc_buffer_t b;
    const uint8_t *tx;
    uint8_t data[SIGGEN_CDC_RX_CAP + 2U];
    uint8_t txdata[3] = { 1U, 2U, 3U };
    siggen_cdc_buffer_init(&b);
    siggen_cdc_buffer_rx(&b, (const uint8_t *)"ignored", 7U);
    assert(siggen_cdc_buffer_getchar(&b) == -1);
    siggen_cdc_buffer_connected(&b, true);

    siggen_cdc_buffer_rx(&b, (const uint8_t *)"ab", 2U);
    siggen_cdc_buffer_rx(&b, (const uint8_t *)"\r\ncd", 4U);
    assert(siggen_cdc_buffer_getchar(&b) == 'a');
    assert(siggen_cdc_buffer_getchar(&b) == 'b');
    assert(siggen_cdc_buffer_getchar(&b) == '\r');
    assert(siggen_cdc_buffer_getchar(&b) == '\n');
    assert(siggen_cdc_buffer_getchar(&b) == 'c');
    assert(siggen_cdc_buffer_getchar(&b) == 'd');
    assert(siggen_cdc_buffer_getchar(&b) == -1);

    for (size_t i = 0U; i < sizeof(data); ++i) data[i] = (uint8_t)'x';
    data[sizeof(data) - 1U] = (uint8_t)'\r';
    siggen_cdc_buffer_rx(&b, data, sizeof(data));
    assert(siggen_cdc_buffer_getchar(&b) == -2);
    assert(siggen_cdc_buffer_getchar(&b) == -1);
    assert(siggen_cdc_buffer_take_session_lost(&b));
    siggen_cdc_buffer_rx(&b, (const uint8_t *)"status\n", 7U);
    assert(siggen_cdc_buffer_getchar(&b) == 's');
    assert(siggen_cdc_buffer_getchar(&b) == 't');
    assert(siggen_cdc_buffer_getchar(&b) == 'a');
    assert(siggen_cdc_buffer_getchar(&b) == 't');
    assert(siggen_cdc_buffer_getchar(&b) == 'u');
    assert(siggen_cdc_buffer_getchar(&b) == 's');
    assert(siggen_cdc_buffer_getchar(&b) == '\n');
    for (size_t i = 0U; i < sizeof(data); ++i) data[i] = (uint8_t)'x';
    data[sizeof(data) - 1U] = (uint8_t)'x';
    siggen_cdc_buffer_rx(&b, data, sizeof(data));
    assert(siggen_cdc_buffer_getchar(&b) == -2);
    siggen_cdc_buffer_rx(&b, (const uint8_t *)"danger\n", 7U);
    assert(siggen_cdc_buffer_getchar(&b) == -1);

    /* The byte that overflows the full ring can itself be the CR boundary. */
    for (unsigned crlf = 0U; crlf < 2U; ++crlf) {
        siggen_cdc_buffer_rx(&b, data, SIGGEN_CDC_RX_CAP);
        siggen_cdc_buffer_rx(&b, (const uint8_t *)"\r", 1U);
        if (crlf != 0U) siggen_cdc_buffer_rx(&b, (const uint8_t *)"\n", 1U);
        siggen_cdc_buffer_rx(&b, (const uint8_t *)"status\n", 7U);
        assert(siggen_cdc_buffer_getchar(&b) == -2);
        const char expected[] = "status\n";
        for (size_t i = 0U; i < sizeof(expected) - 1U; ++i)
            assert(siggen_cdc_buffer_getchar(&b) == expected[i]);
        assert(siggen_cdc_buffer_getchar(&b) == -1);
    }

    assert(!siggen_cdc_buffer_can_write(&b));
    assert(siggen_cdc_buffer_take_session_lost(&b));
    assert(siggen_cdc_buffer_write(&b, txdata, sizeof(txdata)));
    assert(!siggen_cdc_buffer_can_write(&b));
    siggen_cdc_buffer_rx(&b, data, sizeof(data));
    assert(!siggen_cdc_buffer_can_write(&b));
    assert(siggen_cdc_buffer_take_session_lost(&b));
    assert(siggen_cdc_buffer_getchar(&b) == -2);
    assert(!siggen_cdc_buffer_write(&b, txdata, sizeof(txdata)));
    assert(b.txreject == 1U);
    tx = siggen_cdc_buffer_tx_data(&b);
    assert(siggen_cdc_buffer_tx_length(&b) == 3U && tx[0] == 1U && tx[2] == 3U);
    siggen_cdc_buffer_connected(&b, false);
    assert(!siggen_cdc_buffer_can_write(&b));
    assert(siggen_cdc_buffer_take_session_lost(&b));
    assert(!siggen_cdc_buffer_take_session_lost(&b));
    assert(siggen_cdc_buffer_getchar(&b) == -1);
    siggen_cdc_buffer_connected(&b, true);
    assert(!siggen_cdc_buffer_take_session_lost(&b));
    assert(siggen_cdc_buffer_write(&b, txdata, sizeof(txdata)));
    siggen_cdc_buffer_tx_done(&b);
    assert(siggen_cdc_buffer_can_write(&b));
    return 0;
}
