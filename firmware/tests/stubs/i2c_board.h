/* Native register/FIFO substitute. It exercises the actual i2c_port.c state
 * machine, not electrical behavior or HPM silicon W1C/IRQ timing. */
#ifndef TEST_I2C_BOARD_H
#define TEST_I2C_BOARD_H
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
typedef struct { uint32_t INTEN, STATUS, ADDR, DATA, CTRL, CMD, SETUP, TPM; } I2C_Type;
extern I2C_Type test_regs[2];
extern uint64_t test_now;
extern unsigned test_resets[2];
extern uint8_t test_fifo[2][4], test_count[2], test_lines[2];
extern bool test_gpio[2], test_scl_low[2], test_sda_low[2];
typedef struct { I2C_Type *base; uint32_t irq; } bridge_board_port_t;
extern const bridge_board_port_t bridge_board_ports[2];
void bridge_board_mux(uint8_t, bool);
void bridge_board_drive(uint8_t, bool, bool);
uint8_t bridge_board_lines(uint8_t);
#define IRQn_I2C0 0
#define IRQn_I2C3 1
#define CSR_MSTATUS_MIE_MASK 8U
#define SDK_DECLARE_EXT_ISR_M(irq, name)
#define I2C_SOC_FIFO_SIZE 4U
#define I2C_STATUS_LINESCL_MASK (1U << 14)
#define I2C_STATUS_BUSBUSY_MASK (1U << 11)
#define I2C_STATUS_ACK_MASK (1U << 10)
#define I2C_STATUS_CMPL_MASK (1U << 9)
#define I2C_STATUS_BYTERECV_MASK (1U << 8)
#define I2C_STATUS_BYTETRANS_MASK (1U << 7)
#define I2C_STATUS_ADDRHIT_MASK (1U << 3)
#define I2C_STATUS_ARBLOSE_MASK (1U << 4)
#define I2C_EVENT_TRANSACTION_COMPLETE I2C_STATUS_CMPL_MASK
#define I2C_EVENT_BYTE_RECEIVED I2C_STATUS_BYTERECV_MASK
#define I2C_EVENT_ADDRESS_HIT I2C_STATUS_ADDRHIT_MASK
#define I2C_EVENT_FIFO_EMPTY 1U
#define I2C_EVENT_ALL_MASK 0x3FFU
#define I2C_CTRL_PHASE_START_MASK (1U << 12)
#define I2C_CTRL_PHASE_ADDR_MASK (1U << 11)
#define I2C_CTRL_PHASE_DATA_MASK (1U << 10)
#define I2C_CTRL_PHASE_STOP_MASK (1U << 9)
#define I2C_CTRL_DIR_SET(x) ((uint32_t)(x) << 8)
#define I2C_CTRL_DATACNT_SET(x) ((uint32_t)(x) & 0xFFU)
#define I2C_DATACNT_MAP(x) ((x) == 256 ? 0U : (uint32_t)(x))
#define I2C_DIR_SLAVE_WRITE 1U
#define I2C_SETUP_T_SP_SET(x) ((uint32_t)(x) << 21)
#define I2C_SETUP_T_SUDAT_SET(x) ((uint32_t)(x) << 24)
#define I2C_SETUP_T_HDDAT_SET(x) ((uint32_t)(x) << 16)
#define I2C_SETUP_T_SCLRADIO_SET(x) ((uint32_t)(x) << 13)
#define I2C_SETUP_T_SCLHI_SET(x) ((uint32_t)(x) << 4)
#define I2C_SETUP_IICEN_MASK 1U
#define I2C_SETUP_MASTER_MASK 4U
#define I2C_TPM_TPM_SET(x) ((uint32_t)(x))
#define I2C_CMD_ISSUE_DATA_TRANSMISSION 1U
#define clock_mchtmr0 0
enum { status_fail=1, status_timeout=3, status_i2c_no_ack=2001, status_i2c_no_addr_hit=2003, status_i2c_transmit_not_completed=2004 };
static inline unsigned idx(I2C_Type *b) { return (unsigned)(b-test_regs); }
static inline uint32_t clock_get_frequency(int clock) { (void)clock; return 1000000; }
static inline uint32_t board_init_i2c_clock(I2C_Type *b) { (void)b; return 24000000; }
static inline uint32_t disable_global_irq(uint32_t mask) { return mask; }
static inline void restore_global_irq(uint32_t mask) { (void)mask; }
static inline void intc_m_disable_irq(uint32_t irq) { (void)irq; }
static inline void intc_m_enable_irq_with_priority(uint32_t irq,uint32_t priority) { (void)irq; (void)priority; }
static inline void i2c_reset(I2C_Type *b) { ++test_resets[idx(b)]; *b=(I2C_Type){.STATUS=I2C_STATUS_LINESCL_MASK}; test_count[idx(b)]=0; }
static inline void i2c_clear_fifo(I2C_Type *b) { test_count[idx(b)]=0; }
static inline bool i2c_fifo_is_empty(I2C_Type *b) { return test_count[idx(b)]==0; }
static inline bool i2c_fifo_is_full(I2C_Type *b) { return test_count[idx(b)]==4; }
static inline uint8_t i2c_read_byte(I2C_Type *b) {
    unsigned i=idx(b); uint8_t value=test_fifo[i][0];
    --test_count[i]; for(unsigned j=0;j<test_count[i];++j) test_fifo[i][j]=test_fifo[i][j+1];
    return value;
}
static inline void i2c_write_byte(I2C_Type *b,uint8_t value) { unsigned i=idx(b); test_fifo[i][test_count[i]++]=value; }
static inline uint32_t i2c_get_status(I2C_Type *b) { return b->STATUS; }
static inline uint32_t i2c_get_direction(I2C_Type *b) { return (b->CTRL >> 8)&1U; }
static inline uint16_t i2c_get_data_count(I2C_Type *b) { return (uint16_t)(b->CTRL&255U); }
static inline void i2c_clear_status(I2C_Type *b,uint32_t mask) { b->STATUS &= ~mask; }
static inline void i2c_enable_irq(I2C_Type *b,uint32_t mask) { b->INTEN |= mask; }
static inline void i2c_disable_irq(I2C_Type *b,uint32_t mask) { b->INTEN &= ~mask; }
static inline void i2c_respond_ack(I2C_Type *b) { b->CMD=2; }
#endif
