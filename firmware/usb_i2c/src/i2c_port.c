#include "i2c_port.h"
#include "i2c_board.h"
#include "i2c_timing.h"
#include "i2c_table.h"
#include "i2c_deadline.h"
#include "i2c_recovery.h"
#include "hpm_mchtmr_drv.h"
#include <string.h>

enum { MASTER_IDLE, MASTER_WAIT_BUS, MASTER_WRITE, MASTER_READ, MASTER_PROBE, MASTER_STOP };
enum { SLAVE_IDLE, SLAVE_RX, SLAVE_TX };
typedef struct {
    bridge_port_config_t config;
    bridge_i2c_timing_t timing, transfer_timing, prepared_timing;
    uint32_t clock_hz;
    uint64_t config_generation;
    bridge_table_t table;
    volatile uint8_t slave_state, offset, tx_start, tx_next;
    volatile bool rx_first;
    bridge_transfer_t transfer;
    bridge_deadline_t deadline;
    bridge_result_t result;
    uint16_t sent, received;
    uint8_t master_state;
    bool address_hit, transfer_done, fault, recovering;
    bridge_recovery_t recovery;
} port_t;
static port_t ports[BRIDGE_PORT_COUNT];
static uint32_t timer_hz;

/* Read via JTAG for the required bench/WCET record; not a passing assertion. */
volatile uint64_t g_bridge_irq_lock_ticks_max;
volatile uint64_t g_bridge_table_service_us_max;

static uint64_t now_us(void)
{
    uint64_t ticks = mchtmr_get_count(HPM_MCHTMR);
    return ticks / timer_hz * 1000000U + ticks % timer_hz * 1000000U / timer_hz;
}
typedef struct { uint32_t irq; uint64_t ticks; } lock_t;
static lock_t lock(void)
{
    lock_t l;
    l.irq = disable_global_irq(CSR_MSTATUS_MIE_MASK);
    l.ticks = mchtmr_get_count(HPM_MCHTMR);
    __asm__ volatile ("" ::: "memory");
    return l;
}
static void unlock(lock_t l)
{
    __asm__ volatile ("" ::: "memory");
    uint64_t elapsed = mchtmr_get_count(HPM_MCHTMR) - l.ticks;
    if (elapsed > g_bridge_irq_lock_ticks_max) g_bridge_irq_lock_ticks_max = elapsed;
    restore_global_irq(l.irq & CSR_MSTATUS_MIE_MASK);
}
static uint32_t setup_word(const bridge_i2c_timing_t *t, bool master)
{
    return I2C_SETUP_T_SP_SET(t->spike) | I2C_SETUP_T_SUDAT_SET(t->setup) |
        I2C_SETUP_T_HDDAT_SET(t->hold) | I2C_SETUP_T_SCLRADIO_SET(t->ratio - 1U) |
        I2C_SETUP_T_SCLHI_SET(t->high) | I2C_SETUP_IICEN_MASK |
        (master ? I2C_SETUP_MASTER_MASK : 0);
}
static void apply_config(uint8_t port)
{
    port_t *p = &ports[port];
    I2C_Type *b = bridge_board_ports[port].base;
    intc_m_disable_irq(bridge_board_ports[port].irq);
    i2c_reset(b);
    b->TPM = I2C_TPM_TPM_SET(p->timing.multiplier);
    b->ADDR = p->config.slave_address;
    b->SETUP = setup_word(&p->timing, p->config.role == BRIDGE_MASTER);
    if (p->config.role == BRIDGE_SLAVE) {
        b->INTEN = I2C_EVENT_ADDRESS_HIT | I2C_EVENT_TRANSACTION_COMPLETE | I2C_EVENT_BYTE_RECEIVED;
        intc_m_enable_irq_with_priority(bridge_board_ports[port].irq, 2);
    }
}
static bool busy(uint8_t port)
{
    port_t *p = &ports[port];
    return p->master_state != MASTER_IDLE || p->recovering || p->slave_state != SLAVE_IDLE ||
        (bridge_board_ports[port].base->STATUS & (I2C_STATUS_BUSBUSY_MASK | I2C_STATUS_ADDRHIT_MASK));
}
void bridge_ports_init(void)
{
    timer_hz = clock_get_frequency(clock_mchtmr0);
    for (uint8_t i = 0; i < BRIDGE_PORT_COUNT; ++i) {
        port_t *p = &ports[i];
        p->config = (bridge_port_config_t){ BRIDGE_MASTER, 0x50, 100000, 25000, 500000 };
        p->clock_hz = board_init_i2c_clock(bridge_board_ports[i].base);
        if (!bridge_i2c_timing(p->clock_hz, p->config.speed_hz, &p->timing)) {
            p->fault = true;
            continue;
        }
        bridge_board_mux(i, false);
        apply_config(i);
    }
}
void bridge_port_state(uint8_t port, bridge_port_state_t *out)
{
    port_t *p = &ports[port];
    lock_t l = lock();
    *out = (bridge_port_state_t){ .config = p->config, .activity = busy(port) ? 1 : p->fault ? 2 : 0,
        .calculated_hz = p->timing.actual_hz, .config_generation = p->config_generation,
        .table_generation = p->table.generation };
    unlock(l);
}
uint8_t bridge_port_configure(uint8_t port, const bridge_port_config_t *cfg, bool cas, uint64_t expected)
{
    bridge_i2c_timing_t timing;
    if (port >= BRIDGE_PORT_COUNT || cfg->role > BRIDGE_SLAVE || cfg->slave_address < 8 || cfg->slave_address > 0x77 ||
        cfg->stretch_us < 1000 || cfg->stretch_us > 100000 || cfg->transaction_us < 1000 ||
        cfg->transaction_us > 2000000 || cfg->transaction_us < cfg->stretch_us ||
        (cfg->role == BRIDGE_SLAVE && cfg->speed_hz != 100000 && cfg->speed_hz != 400000 && cfg->speed_hz != 1000000) ||
        !bridge_i2c_timing(ports[port].clock_hz, cfg->speed_hz, &timing)) return BRIDGE_BAD_REQUEST;
    port_t *p = &ports[port];
    lock_t l = lock();
    if (cas && expected != p->config_generation) { unlock(l); return BRIDGE_CONFLICT; }
    if (busy(port)) { unlock(l); return BRIDGE_BUSY; }
    /* All fallible validation is complete before the selected controller is
     * changed. No other controller, table or USB state is reset. */
    p->config = *cfg; p->timing = timing;
    p->offset = 0; p->slave_state = SLAVE_IDLE; p->fault = false;
    apply_config(port);
    ++p->config_generation;
    unlock(l);
    return BRIDGE_OK;
}
static void table_resync(port_t *p, uint64_t started)
{
    bool done;
    do {
        lock_t l = lock();
        done = bridge_table_resync_one(&p->table);
        unlock(l);
    } while (!done);
    uint64_t elapsed = now_us() - started;
    if (elapsed > g_bridge_table_service_us_max) g_bridge_table_service_us_max = elapsed;
}
uint8_t bridge_port_table_read(uint8_t port, uint16_t offset, uint16_t len, uint8_t *out, uint64_t *generation)
{
    if (port >= BRIDGE_PORT_COUNT || !len || offset >= 256 || len > 256U - offset) return BRIDGE_BAD_REQUEST;
    port_t *p = &ports[port];
    if (p->config.role != BRIDGE_SLAVE) return BRIDGE_WRONG_ROLE;
    uint64_t started = now_us();
    lock_t l = lock();
    bool ok = bridge_table_snapshot_begin(&p->table, generation);
    unlock(l);
    if (!ok) return BRIDGE_BUSY;
    bridge_table_snapshot_copy(&p->table, offset, len, out);
    l = lock(); bridge_table_resync_begin(&p->table); unlock(l);
    table_resync(p, started);
    return BRIDGE_OK;
}
uint8_t bridge_port_table_write(uint8_t port, uint16_t offset, uint16_t len, const uint8_t *data,
    bool cas, uint64_t expected, uint64_t *generation)
{
    if (port >= BRIDGE_PORT_COUNT || !len || offset >= 256 || len > 256U - offset) return BRIDGE_BAD_REQUEST;
    port_t *p = &ports[port];
    if (p->config.role != BRIDGE_SLAVE) return BRIDGE_WRONG_ROLE;
    uint64_t started = now_us();
    lock_t l = lock();
    if (cas && expected != p->table.generation) { unlock(l); return BRIDGE_CONFLICT; }
    bool ok = bridge_table_patch_begin(&p->table, offset, len, cas, expected);
    unlock(l);
    if (!ok) return BRIDGE_BUSY;
    bridge_table_patch_copy(&p->table, data);
    l = lock(); ok = bridge_table_patch_commit(&p->table, cas, expected, generation); unlock(l);
    table_resync(p, started);
    return ok ? BRIDGE_OK : BRIDGE_CONFLICT;
}

static void slave_rx(port_t *p, I2C_Type *b)
{
    /* Hardware FIFO size is four: no unbounded ISR drain. */
    for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && !i2c_fifo_is_empty(b); ++i) {
        uint8_t value = i2c_read_byte(b);
        if (p->rx_first) { p->offset = value; p->rx_first = false; }
        else bridge_table_external_write(&p->table, p->offset++, value);
    }
}
static void slave_tx(port_t *p, I2C_Type *b)
{
    for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && !i2c_fifo_is_full(b); ++i)
        i2c_write_byte(b, bridge_table_read_byte(&p->table, p->tx_next++));
}
static void slave_irq(uint8_t port)
{
    port_t *p = &ports[port];
    I2C_Type *b = bridge_board_ports[port].base;
    uint32_t st = i2c_get_status(b);
    if (p->config.role != BRIDGE_SLAVE) { b->INTEN = 0; return; }
    if (st & I2C_EVENT_ADDRESS_HIT) {
        if (p->slave_state == SLAVE_RX) slave_rx(p, b);
        if (i2c_get_direction(b) == I2C_DIR_SLAVE_WRITE) {
            p->slave_state = SLAVE_TX; p->tx_start = p->offset; p->tx_next = p->offset;
            slave_tx(p, b); i2c_enable_irq(b, I2C_EVENT_FIFO_EMPTY);
        } else {
            /* No TX-empty interrupt should remain armed in a receive phase.
             * The preceding TX CMPL clears its unsent FIFO prefetch. */
            i2c_disable_irq(b, I2C_EVENT_FIFO_EMPTY);
            p->slave_state = SLAVE_RX; p->rx_first = true;
        }
        i2c_clear_status(b, I2C_EVENT_ADDRESS_HIT);
    }
    if (st & I2C_EVENT_BYTE_RECEIVED) {
        i2c_clear_status(b, I2C_EVENT_BYTE_RECEIVED);
        if (p->slave_state == SLAVE_RX) slave_rx(p, b);
        /* BYTERECV disables hardware auto-ACK. */
        i2c_respond_ack(b);
    }
    if ((st & I2C_EVENT_FIFO_EMPTY) && p->slave_state == SLAVE_TX) slave_tx(p, b);
    if (st & I2C_EVENT_TRANSACTION_COMPLETE) {
        /* HPM CMPL gates the next transaction until cleared. ADDRHIT+CMPL
         * in this snapshot can describe one short transaction; complete it
         * after its address/data work, never carry its FIFO into the next. */
        if (p->slave_state == SLAVE_RX) slave_rx(p, b);
        else if (p->slave_state == SLAVE_TX) {
            i2c_disable_irq(b, I2C_EVENT_FIFO_EMPTY);
            p->offset = (uint8_t)(p->tx_start + i2c_get_data_count(b));
            i2c_clear_fifo(b);
        }
        p->slave_state = SLAVE_IDLE;
        i2c_clear_status(b, I2C_EVENT_TRANSACTION_COMPLETE);
    }
}
SDK_DECLARE_EXT_ISR_M(IRQn_I2C0, bridge_i2c0_isr)
void bridge_i2c0_isr(void) { slave_irq(0); }
SDK_DECLARE_EXT_ISR_M(IRQn_I2C3, bridge_i2c3_isr)
void bridge_i2c3_isr(void) { slave_irq(1); }

uint8_t bridge_port_transfer_validate(const bridge_transfer_t *t)
{
    bridge_i2c_timing_t timing;
    if (t->port >= BRIDGE_PORT_COUNT || t->address > 0x7F || t->write_len > 256 || t->read_len > 256 ||
        (!t->probe && !t->write_len && !t->read_len) ||
        (t->probe && (t->address < 8 || t->address > 0x77 || t->write_len || t->read_len)) ||
        !bridge_i2c_timing(ports[t->port].clock_hz, t->speed_hz, &timing)) return BRIDGE_BAD_REQUEST;
    port_t *p = &ports[t->port];
    if (p->config.role != BRIDGE_MASTER) return BRIDGE_WRONG_ROLE;
    if (p->master_state != MASTER_IDLE || p->recovering) return BRIDGE_BUSY;
    p->prepared_timing = timing;
    return BRIDGE_OK;
}
static void master_phase(port_t *p, I2C_Type *b, bool read)
{
    uint16_t count = read ? p->transfer.read_len : p->transfer.write_len;
    p->address_hit = false;
    b->STATUS = I2C_EVENT_ALL_MASK;
    i2c_clear_fifo(b);
    b->ADDR = p->transfer.address;
    b->CTRL = I2C_CTRL_PHASE_START_MASK | I2C_CTRL_PHASE_ADDR_MASK |
        ((read || !p->transfer.read_len) ? I2C_CTRL_PHASE_STOP_MASK : 0) |
        (p->transfer.probe ? 0 : I2C_CTRL_PHASE_DATA_MASK) |
        I2C_CTRL_DIR_SET(read) | I2C_CTRL_DATACNT_SET(I2C_DATACNT_MAP(count));
    p->master_state = p->transfer.probe ? MASTER_PROBE : read ? MASTER_READ : MASTER_WRITE;
    if (!read && !p->transfer.probe) {
        for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && p->sent < count && !i2c_fifo_is_full(b); ++i)
            i2c_write_byte(b, p->transfer.write_data[p->sent++]);
    }
    b->CMD = I2C_CMD_ISSUE_DATA_TRANSMISSION;
}
void bridge_port_transfer_start(const bridge_transfer_t *t)
{
    port_t *p = &ports[t->port];
    p->transfer = *t; p->sent = 0; p->received = 0; p->transfer_done = false;
    p->deadline = (bridge_deadline_t){ .started = now_us() };
    /* Prepared for both batch ports before either START. No slow timing
     * search between the two hardware starts. */
    p->transfer_timing = p->prepared_timing;
    p->master_state = MASTER_WAIT_BUS;
    I2C_Type *b = bridge_board_ports[t->port].base;
    b->STATUS = I2C_STATUS_BYTERECV_MASK | I2C_STATUS_BYTETRANS_MASK;
    if (!(b->STATUS & I2C_STATUS_BUSBUSY_MASK)) {
        b->TPM = I2C_TPM_TPM_SET(p->transfer_timing.multiplier);
        b->SETUP = setup_word(&p->transfer_timing, true);
        master_phase(p, b, !t->write_len && !t->probe);
    }
}
static void master_finish(uint8_t port, uint8_t status, uint32_t hpm)
{
    port_t *p = &ports[port];
    p->result = (bridge_result_t){ status, hpm };
    p->master_state = MASTER_IDLE; p->transfer_done = true; p->fault = status != BRIDGE_OK && status != BRIDGE_NACK;
    /* Abort a fault and release this controller; an explicit recovery command
     * is required for a physically stuck bus. Never replay the write. */
    apply_config(port);
}
static void master_nack(uint8_t port, uint32_t hpm)
{
    port_t *p = &ports[port];
    I2C_Type *b = bridge_board_ports[port].base;
    if (!(b->CTRL & I2C_CTRL_PHASE_STOP_MASK)) {
        /* The write half of a combined request deliberately had no STOP.
         * On NACK send that STOP explicitly, within the same total deadline. */
        p->result = (bridge_result_t){ BRIDGE_NACK, hpm };
        p->master_state = MASTER_STOP;
        b->STATUS = I2C_STATUS_CMPL_MASK;
        b->CTRL = I2C_CTRL_PHASE_STOP_MASK;
        b->CMD = I2C_CMD_ISSUE_DATA_TRANSMISSION;
    } else master_finish(port, BRIDGE_NACK, hpm);
}
static void master_poll(uint8_t port, uint64_t now)
{
    port_t *p = &ports[port];
    I2C_Type *b = bridge_board_ports[port].base;
    uint32_t st = b->STATUS;
    bool progress = (st & (I2C_STATUS_BYTERECV_MASK | I2C_STATUS_BYTETRANS_MASK)) != 0;
    if (bridge_deadline_step(&p->deadline, now, (st & I2C_STATUS_LINESCL_MASK) != 0, progress,
        p->config.transaction_us, p->config.stretch_us, (p->transfer_timing.low_ns + 999U) / 1000U)) {
        master_finish(port, BRIDGE_TIMEOUT, status_timeout); return;
    }
    if (st & I2C_STATUS_ARBLOSE_MASK) { master_finish(port, BRIDGE_I2C_ERROR, status_fail); return; }
    if (p->master_state == MASTER_STOP) {
        if (st & I2C_STATUS_CMPL_MASK) master_finish(port, p->result.status, p->result.hpm_status);
        return;
    }
    if (p->master_state == MASTER_WAIT_BUS) {
        if (!(st & I2C_STATUS_BUSBUSY_MASK)) {
            b->TPM = I2C_TPM_TPM_SET(p->transfer_timing.multiplier);
            b->SETUP = setup_word(&p->transfer_timing, true);
            master_phase(p, b, !p->transfer.write_len && !p->transfer.probe);
        }
        return;
    }
    if (st & I2C_STATUS_ADDRHIT_MASK) p->address_hit = true;
    b->STATUS = st & (I2C_STATUS_BYTERECV_MASK | I2C_STATUS_BYTETRANS_MASK | I2C_STATUS_ADDRHIT_MASK);
    if (p->master_state == MASTER_WRITE) {
        for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && p->sent < p->transfer.write_len && !i2c_fifo_is_full(b); ++i)
            i2c_write_byte(b, p->transfer.write_data[p->sent++]);
    } else if (p->master_state == MASTER_READ) {
        for (unsigned i = 0; i < I2C_SOC_FIFO_SIZE && p->received < p->transfer.read_len && !i2c_fifo_is_empty(b); ++i)
            p->transfer.read_data[p->received++] = i2c_read_byte(b);
    }
    if (!(st & I2C_STATUS_CMPL_MASK)) return;
    if (!p->address_hit) { master_nack(port, status_i2c_no_addr_hit); return; }
    if (p->master_state == MASTER_WRITE && (!(st & I2C_STATUS_ACK_MASK) || i2c_get_data_count(b) != 0)) {
        master_nack(port, status_i2c_no_ack); return;
    }
    if (p->master_state == MASTER_READ && p->received != p->transfer.read_len) {
        master_finish(port, BRIDGE_I2C_ERROR, status_i2c_transmit_not_completed); return;
    }
    if (p->master_state == MASTER_WRITE && p->transfer.read_len) {
        master_phase(p, b, true); return;
    }
    master_finish(port, BRIDGE_OK, 0);
}
bool bridge_port_transfer_result(uint8_t port, bridge_result_t *result)
{
    if (!ports[port].transfer_done) return false;
    *result = ports[port].result; return true;
}
uint8_t bridge_port_recover_start(uint8_t port)
{
    if (port >= BRIDGE_PORT_COUNT) return BRIDGE_BAD_REQUEST;
    port_t *p = &ports[port];
    if (p->config.role != BRIDGE_MASTER) return BRIDGE_WRONG_ROLE;
    if (p->master_state != MASTER_IDLE || p->recovering) return BRIDGE_BUSY;
    uint8_t initial_lines = bridge_board_lines(port);
    i2c_reset(bridge_board_ports[port].base);
    bridge_board_mux(port, true);
    bridge_recovery_begin(&p->recovery, now_us(), initial_lines, p->config.stretch_us);
    p->recovering = true;
    return BRIDGE_OK;
}
bool bridge_port_recover_result(uint8_t port, bridge_recovery_result_t *out)
{
    port_t *p = &ports[port];
    if (p->recovering || p->recovery.state != REC_DONE) return false;
    *out = (bridge_recovery_result_t){ p->recovery.clocks, p->recovery.initial_lines, p->recovery.final_lines, p->recovery.reason };
    return true;
}
void bridge_ports_poll(void)
{
    uint64_t now = now_us();
    for (uint8_t i = 0; i < BRIDGE_PORT_COUNT; ++i) {
        port_t *p = &ports[i];
        if (p->master_state != MASTER_IDLE) master_poll(i, now);
        if (p->recovering) {
            bool done = bridge_recovery_step(&p->recovery, now, bridge_board_lines(i));
            bridge_board_drive(i, p->recovery.scl_low, p->recovery.sda_low);
            if (done) {
                bridge_board_mux(i, false); apply_config(i);
                p->recovering = false; p->fault = p->recovery.reason != 0;
            }
        }
    }
}
void bridge_ports_abort(void)
{
    for (uint8_t i = 0; i < BRIDGE_PORT_COUNT; ++i) {
        port_t *p = &ports[i];
        if (p->master_state != MASTER_IDLE) master_finish(i, BRIDGE_I2C_ERROR, status_fail);
        if (p->recovering) {
            bridge_board_drive(i, false, false); bridge_board_mux(i, false); apply_config(i);
            p->recovering = false; p->recovery.state = REC_DONE; p->recovery.reason = 3; p->fault = true;
        }
        /* Table commands complete their bounded RESYNC before returning to
         * USB task, so disconnect never leaves an excluded mirror behind. */
    }
}
