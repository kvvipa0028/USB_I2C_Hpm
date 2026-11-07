#ifndef SIGGEN_TEST_BOARD_H
#define SIGGEN_TEST_BOARD_H
#include <stdint.h>
#define IRQn_GPIO0_B 2U
#define SDK_DECLARE_EXT_ISR_M(irq, handler)
void intc_m_enable_irq_with_priority(uint32_t irq, uint32_t priority);
#endif
