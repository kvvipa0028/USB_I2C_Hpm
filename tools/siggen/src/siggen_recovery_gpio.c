#include "siggen_recovery_gpio.h"
#include "siggen_board.h"
#include "board.h"
#include "hpm_gpio_drv.h"

/* HPM6750EVKMini: PB11=SCL/P1[13], PB10=SDA/P1[15]. */
static siggen_peer_t *owner;
static uint32_t owner_generation;

void siggen_recovery_gpio_disarm(void)
{
    owner = NULL;
    owner_generation = 0;
    gpio_disable_pin_interrupt(HPM_GPIO0, GPIO_DI_GPIOB, 11);
    gpio_clear_pin_interrupt_flag(HPM_GPIO0, GPIO_DI_GPIOB, 11);
}

bool siggen_recovery_gpio_arm(siggen_peer_t *peer, uint32_t generation)
{
    siggen_recovery_gpio_disarm();
    if (peer == NULL || generation == 0 || !gpio_read_pin(HPM_GPIO0, GPIO_DI_GPIOB, 11)) return false;
    gpio_config_pin_interrupt(HPM_GPIO0, GPIO_DI_GPIOB, 11, gpio_interrupt_trigger_edge_rising);
    /* The starting high level is not a pulse; discard configuration/stale IF. */
    gpio_clear_pin_interrupt_flag(HPM_GPIO0, GPIO_DI_GPIOB, 11);
    owner = peer;
    owner_generation = generation;
    gpio_enable_pin_interrupt(HPM_GPIO0, GPIO_DI_GPIOB, 11);
    intc_m_enable_irq_with_priority(IRQn_GPIO0_B, 1);
    return true;
}

SDK_DECLARE_EXT_ISR_M(IRQn_GPIO0_B, siggen_scl_rising_isr)
void siggen_scl_rising_isr(void)
{
    if (!gpio_check_pin_interrupt_flag(HPM_GPIO0, GPIO_DI_GPIOB, 11) ||
        !gpio_check_pin_interrupt_enabled(HPM_GPIO0, GPIO_DI_GPIOB, 11)) return;
    gpio_clear_pin_interrupt_flag(HPM_GPIO0, GPIO_DI_GPIOB, 11);
    if (owner != NULL && siggen_peer_scl_rising(owner, owner_generation, siggen_board_now_us())) {
        /* Release only. No pinmux/configure, formatting, or wait in the ISR. */
        gpio_set_pin_input(HPM_GPIO0, GPIO_DO_GPIOB, 10);
        siggen_recovery_gpio_disarm();
    }
}
