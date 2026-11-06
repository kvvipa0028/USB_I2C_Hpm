#include "siggen_board.h"
#include "board.h"
#include "hpm_i2c_drv.h"
#include "hpm_gpio_drv.h"
#include "hpm_mchtmr_drv.h"

/* SDK v1.13.0 hpm6750evkmini: PB11=P1[13] SCL, PB10=P1[15] SDA.
 * This target intentionally does not infer pins for other EVKs or EVT. */
#define PEER_I2C HPM_I2C0
#define PEER_IRQ IRQn_I2C0
static siggen_peer_t *peer;
static uint32_t source_hz, timer_hz;
static bool master_address_hit;
static uint8_t master_received;

uint32_t siggen_board_lock(void)
{
    uint32_t irq = disable_global_irq(CSR_MSTATUS_MIE_MASK);
    __asm__ volatile ("" ::: "memory");
    return irq;
}
void siggen_board_unlock(uint32_t irq)
{
    __asm__ volatile ("" ::: "memory");
    restore_global_irq(irq & CSR_MSTATUS_MIE_MASK);
}
uint64_t siggen_board_now_us(void)
{
    uint64_t n = mchtmr_get_count(HPM_MCHTMR);
    return n / timer_hz * 1000000U + n % timer_hz * 1000000U / timer_hz;
}
static void drive(uint8_t mask)
{
    /* Output latches stay zero. Input releases the pad; never drive high. */
    gpio_write_pin(HPM_GPIO0, GPIO_DO_GPIOB, 11, 0);
    gpio_write_pin(HPM_GPIO0, GPIO_DO_GPIOB, 10, 0);
    if ((mask & 1U) != 0) gpio_set_pin_output(HPM_GPIO0, GPIO_DO_GPIOB, 11);
    else gpio_set_pin_input(HPM_GPIO0, GPIO_DO_GPIOB, 11);
    if ((mask & 2U) != 0) gpio_set_pin_output(HPM_GPIO0, GPIO_DO_GPIOB, 10);
    else gpio_set_pin_input(HPM_GPIO0, GPIO_DO_GPIOB, 10);
}
static void disable(void *context)
{
    (void)context;
    intc_m_disable_irq(PEER_IRQ);
    PEER_I2C->INTEN = 0;
    i2c_reset(PEER_I2C);
}
static void unhold(void *context)
{
    (void)context;
    drive(0);
    init_i2c_pins(PEER_I2C);
}
static void hold(void *context, uint8_t mask)
{
    disable(context);
    drive(0);
    HPM_IOC->PAD[IOC_PAD_PB11].PAD_CTL = IOC_PAD_PAD_CTL_OD_MASK;
    HPM_IOC->PAD[IOC_PAD_PB10].PAD_CTL = IOC_PAD_PAD_CTL_OD_MASK;
    HPM_IOC->PAD[IOC_PAD_PB11].FUNC_CTL = IOC_PB11_FUNC_CTL_GPIO_B_11;
    HPM_IOC->PAD[IOC_PAD_PB10].FUNC_CTL = IOC_PB10_FUNC_CTL_GPIO_B_10;
    drive(mask);
}
static bool configure(void *context, bool slave, uint8_t address, uint32_t hz, bool address_nack)
{
    disable(context);
    unhold(context);
    i2c_config_t config = { .is_10bit_addressing = false,
        .i2c_mode = hz == 100000 ? i2c_mode_normal : hz == 400000 ? i2c_mode_fast : i2c_mode_fast_plus };
    hpm_stat_t result = slave ? i2c_init_slave(PEER_I2C, source_hz, &config, address) :
        i2c_init_master(PEER_I2C, source_hz, &config);
    if (result != status_success) { disable(context); return false; }
    if (slave) {
        if (address_nack) {
            /* Address-absence injection, not a programmable address-phase ACK. */
            disable(context);
        } else {
            PEER_I2C->INTEN = I2C_EVENT_ADDRESS_HIT | I2C_EVENT_TRANSACTION_COMPLETE | I2C_EVENT_BYTE_RECEIVED;
            intc_m_enable_irq_with_priority(PEER_IRQ, 1);
        }
    }
    return true;
}
static bool busy(void *context)
{
    (void)context;
    return (PEER_I2C->STATUS & (I2C_STATUS_BUSBUSY_MASK | I2C_STATUS_ADDRHIT_MASK)) != 0;
}
static uint8_t lines(void *context)
{
    (void)context;
    return (uint8_t)((gpio_read_pin(HPM_GPIO0, GPIO_DI_GPIOB, 11) ? 1U : 0U) |
                     (gpio_read_pin(HPM_GPIO0, GPIO_DI_GPIOB, 10) ? 2U : 0U));
}
static void ack(void *context, bool acknowledge)
{
    (void)context;
    if (acknowledge) i2c_respond_ack(PEER_I2C);
    else i2c_respond_Nack(PEER_I2C);
}
static bool ack_valid(void *context)
{
    (void)context;
    uint32_t status = PEER_I2C->STATUS;
    return (status & I2C_STATUS_BUSBUSY_MASK) != 0 &&
        (status & (I2C_STATUS_CMPL_MASK | I2C_STATUS_ADDRHIT_MASK)) == 0;
}
static bool master_begin(void *context, bool read, uint8_t address)
{
    if (busy(context)) return false;
    static const uint8_t bytes[] = { 0xde, 0xad, 0x00 };
    master_address_hit = false;
    master_received = 0;
    PEER_I2C->STATUS = I2C_EVENT_ALL_MASK;
    i2c_clear_fifo(PEER_I2C);
    PEER_I2C->ADDR = address;
    PEER_I2C->CTRL = I2C_CTRL_PHASE_START_MASK | I2C_CTRL_PHASE_ADDR_MASK |
        I2C_CTRL_PHASE_DATA_MASK | I2C_CTRL_PHASE_STOP_MASK | I2C_CTRL_DIR_SET(read) |
        I2C_CTRL_DATACNT_SET(read ? 2U : 3U);
    if (!read) for (unsigned i = 0; i < sizeof(bytes); ++i) i2c_write_byte(PEER_I2C, bytes[i]);
    PEER_I2C->CMD = I2C_CMD_ISSUE_DATA_TRANSMISSION;
    return true;
}
static bool master_poll(void *context, bool read, siggen_result_t *result, uint8_t data[2])
{
    (void)context;
    uint32_t status = PEER_I2C->STATUS;
    if ((status & I2C_STATUS_ARBLOSE_MASK) != 0) {
        *result = (siggen_result_t){ SIG_RESULT_ERROR, status_fail };
        i2c_reset(PEER_I2C);
        return true;
    }
    if ((status & I2C_STATUS_ADDRHIT_MASK) != 0) master_address_hit = true;
    PEER_I2C->STATUS = status & (I2C_STATUS_BYTERECV_MASK | I2C_STATUS_BYTETRANS_MASK | I2C_STATUS_ADDRHIT_MASK);
    if (read) for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && master_received < 2 && !i2c_fifo_is_empty(PEER_I2C); ++i)
        data[master_received++] = i2c_read_byte(PEER_I2C);
    if ((status & I2C_STATUS_CMPL_MASK) == 0) return false;
    if (!master_address_hit) *result = (siggen_result_t){ SIG_RESULT_ADDRESS_NACK, status_i2c_no_addr_hit };
    else if (!read && ((status & I2C_STATUS_ACK_MASK) == 0 || i2c_get_data_count(PEER_I2C) != 0))
        *result = (siggen_result_t){ SIG_RESULT_DATA_NACK, status_i2c_no_ack };
    else if (read && master_received != 2)
        *result = (siggen_result_t){ SIG_RESULT_ERROR, status_i2c_transmit_not_completed };
    else *result = (siggen_result_t){ SIG_RESULT_OK, 0 };
    return true;
}
static const siggen_hal_t hal = { .context = NULL, .configure = configure, .disable = disable,
    .busy = busy, .hold = hold, .unhold = unhold, .lines = lines, .ack = ack,
    .ack_valid = ack_valid, .master_begin = master_begin, .master_poll = master_poll };
const siggen_hal_t *siggen_board_hal(void) { return &hal; }
void siggen_board_init(siggen_peer_t *p)
{
    peer = p;
    timer_hz = clock_get_frequency(clock_mchtmr0);
    source_hz = board_init_i2c_clock(PEER_I2C);
    /* Avoid board_init_i2c(): its startup bus-clear is not an injected event. */
    init_i2c_pins(PEER_I2C);
    disable(NULL);
}
static void tx_fill(void)
{
    for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && !i2c_fifo_is_full(PEER_I2C); ++i)
        i2c_write_byte(PEER_I2C, siggen_peer_transmit(peer));
}
SDK_DECLARE_EXT_ISR_M(PEER_IRQ, siggen_i2c_isr)
void siggen_i2c_isr(void)
{
    uint32_t status = i2c_get_status(PEER_I2C);
    if (peer->mode != SIG_MODE_SLAVE || peer->address_nack || peer->held_mask != 0) {
        PEER_I2C->INTEN = 0;
        return;
    }
    if ((status & I2C_EVENT_ADDRESS_HIT) != 0) {
        bool read = i2c_get_direction(PEER_I2C) == I2C_DIR_SLAVE_WRITE;
        siggen_peer_address(peer, read);
        if (read) { tx_fill(); i2c_enable_irq(PEER_I2C, I2C_EVENT_FIFO_EMPTY); }
        else i2c_disable_irq(PEER_I2C, I2C_EVENT_FIFO_EMPTY);
        i2c_clear_status(PEER_I2C, I2C_EVENT_ADDRESS_HIT);
    }
    if ((status & I2C_EVENT_BYTE_RECEIVED) != 0) {
        i2c_clear_status(PEER_I2C, I2C_EVENT_BYTE_RECEIVED);
        /* Manual ACK halts progress per received byte; no ISR delay loop. */
        enum siggen_ack reply = SIG_NACK_NOW;
        if (!i2c_fifo_is_empty(PEER_I2C)) reply = siggen_peer_receive(peer, i2c_read_byte(PEER_I2C), siggen_board_now_us());
        if (reply != SIG_ACK_DEFER) ack(NULL, reply == SIG_ACK_NOW);
    }
    if ((status & I2C_EVENT_FIFO_EMPTY) != 0 && i2c_get_direction(PEER_I2C) == I2C_DIR_SLAVE_WRITE) tx_fill();
    if ((status & I2C_EVENT_TRANSACTION_COMPLETE) != 0) {
        i2c_disable_irq(PEER_I2C, I2C_EVENT_FIFO_EMPTY);
        siggen_peer_complete(peer, i2c_get_data_count(PEER_I2C));
        i2c_clear_fifo(PEER_I2C);
        i2c_clear_status(PEER_I2C, I2C_EVENT_TRANSACTION_COMPLETE);
    }
}
