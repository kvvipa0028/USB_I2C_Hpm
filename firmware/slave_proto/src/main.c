/*
 * slave_proto (T1.9): I2C slave emulating an EEPROM-style register table
 * (PRD FR-SLV-01~03) on HPM6300EVK I2C0, interrupt driven, speed SLV_MODE.
 *
 * Table: 256 bytes, 1-byte offset. In a write transaction the first byte
 * sets the offset, the following bytes are stored at offset, offset+1, ...
 * A read transaction sends from the current offset. The offset advances by
 * one per byte and wraps from 0xFF to 0x00.
 *
 * Peer: tools/slave_test on HPM6750EVKMini (external master).
 * Results are read over JTAG: g_slave, g_table.
 */

#include "board.h"
#include "hpm_i2c_drv.h"

#define SLV_I2C   BOARD_APP_I2C_BASE
#define SLV_IRQ   BOARD_APP_I2C_IRQ
#define SLV_ADDR  0x50U
#ifndef SLV_MODE
#define SLV_MODE  i2c_mode_normal   /* normal = 100k, fast = 400k, fast_plus = 1M */
#endif                            /* match BUS_MODE in slave_test */

volatile struct {
    uint32_t magic;      /* 0x51A7E001 = slave initialized */
    uint32_t addr_hits;
    uint32_t rx_done;    /* completed master-write transactions */
    uint32_t tx_done;    /* completed master-read transactions */
} g_slave;

volatile uint8_t g_table[256];

enum { ST_IDLE, ST_RX, ST_TX };
static uint8_t s_state;
static uint8_t s_off;        /* current offset */
static uint8_t s_tx_start;   /* offset at the start of the current read */
static uint8_t s_tx_next;    /* next table byte to load into the FIFO */
static bool s_rx_first;      /* next received byte is the offset */

static void rx_drain(void)
{
    while (!i2c_fifo_is_empty(SLV_I2C)) {
        uint8_t b = i2c_read_byte(SLV_I2C);
        if (s_rx_first) {
            s_off = b;
            s_rx_first = false;
        } else {
            g_table[s_off++] = b;
        }
    }
}

static void tx_fill(void)
{
    while (!i2c_fifo_is_full(SLV_I2C)) {
        i2c_write_byte(SLV_I2C, g_table[s_tx_next++]);
    }
}

SDK_DECLARE_EXT_ISR_M(SLV_IRQ, slave_isr)
void slave_isr(void)
{
    uint32_t st = i2c_get_status(SLV_I2C);

    /* the controller holds the next transaction until CMPL is cleared, so a
     * single status read never mixes two transactions: handle the address
     * phase first and the completion last */
    if (st & I2C_EVENT_ADDRESS_HIT) {
        if (s_state == ST_RX) {
            rx_drain();   /* bytes of the write phase before a repeated START */
        }
        g_slave.addr_hits++;
        if (i2c_get_direction(SLV_I2C) == I2C_DIR_SLAVE_WRITE) {
            s_state = ST_TX;
            s_tx_start = s_off;
            s_tx_next = s_off;
            tx_fill();
            i2c_enable_irq(SLV_I2C, I2C_EVENT_FIFO_EMPTY);
        } else {
            s_state = ST_RX;
            s_rx_first = true;
        }
        i2c_clear_status(SLV_I2C, I2C_EVENT_ADDRESS_HIT);
    }
    if (st & I2C_EVENT_BYTE_RECEIVED) {
        i2c_clear_status(SLV_I2C, I2C_EVENT_BYTE_RECEIVED);
        if (s_state == ST_RX) {
            rx_drain();
        }
        /* with BYTERECV enabled auto-ACK is off (INTEN doc): ACK by hand,
         * which also releases SCL */
        i2c_respond_ack(SLV_I2C);
    }
    if ((st & I2C_EVENT_FIFO_EMPTY) && s_state == ST_TX) {
        tx_fill();
    }
    if (st & I2C_EVENT_TRANSACTION_COMPLETE) {
        if (s_state == ST_RX) {
            rx_drain();
            g_slave.rx_done++;
        } else if (s_state == ST_TX) {
            i2c_disable_irq(SLV_I2C, I2C_EVENT_FIFO_EMPTY);
            /* bytes actually sent; the FIFO may hold unsent prefetch */
            s_off = (uint8_t)(s_tx_start + i2c_get_data_count(SLV_I2C));
            i2c_clear_fifo(SLV_I2C);
            g_slave.tx_done++;
        }
        s_state = ST_IDLE;
        i2c_clear_status(SLV_I2C, I2C_EVENT_TRANSACTION_COMPLETE);
    }
}

int main(void)
{
    i2c_config_t cfg = { .is_10bit_addressing = false, .i2c_mode = SLV_MODE };
    uint32_t freq;

    board_init();
    freq = board_init_i2c_clock(SLV_I2C);
    init_i2c_pins(SLV_I2C);
    if (i2c_init_slave(SLV_I2C, freq, &cfg, SLV_ADDR) != status_success) {
        while (1) {
        }
    }
    intc_m_enable_irq_with_priority(SLV_IRQ, 1);
    i2c_enable_irq(SLV_I2C, I2C_EVENT_ADDRESS_HIT | I2C_EVENT_TRANSACTION_COMPLETE |
                            I2C_EVENT_BYTE_RECEIVED);
    g_slave.magic = 0x51A7E001;

    while (1) {
    }
    return 0;
}
