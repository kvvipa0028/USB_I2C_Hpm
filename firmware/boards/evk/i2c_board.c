#include "i2c_board.h"
/* Verified against SDK v1.13.0 hpm6300evk/pinmux.c and HPM6360 hpm_iomux.h.
 * CH1 connector/pad accessibility and board conflicts remain PENDING bench
 * verification. These are EVK mappings, not an inferred EVT pin assignment. */
const bridge_board_port_t bridge_board_ports[2] = {
    { HPM_I2C0, IRQn_I2C0, IOC_PAD_PC13, IOC_PAD_PC14,
      IOC_PC13_FUNC_CTL_I2C0_SCL, IOC_PC14_FUNC_CTL_I2C0_SDA, 13, 14 },
    { HPM_I2C3, IRQn_I2C3, IOC_PAD_PC11, IOC_PAD_PC12,
      IOC_PC11_FUNC_CTL_I2C3_SCL, IOC_PC12_FUNC_CTL_I2C3_SDA, 11, 12 }
};
void bridge_board_drive(uint8_t port, bool scl_low, bool sda_low)
{
    const bridge_board_port_t *p = &bridge_board_ports[port];
    /* Output latches are always zero. Input means release, never drive high. */
    gpio_write_pin(HPM_GPIO0, GPIO_DO_GPIOC, p->scl_pin, 0);
    gpio_write_pin(HPM_GPIO0, GPIO_DO_GPIOC, p->sda_pin, 0);
    if (scl_low) gpio_set_pin_output(HPM_GPIO0, GPIO_DO_GPIOC, p->scl_pin);
    else gpio_set_pin_input(HPM_GPIO0, GPIO_DO_GPIOC, p->scl_pin);
    if (sda_low) gpio_set_pin_output(HPM_GPIO0, GPIO_DO_GPIOC, p->sda_pin);
    else gpio_set_pin_input(HPM_GPIO0, GPIO_DO_GPIOC, p->sda_pin);
}
void bridge_board_mux(uint8_t port, bool gpio)
{
    const bridge_board_port_t *p = &bridge_board_ports[port];
    bridge_board_drive(port, false, false);
    uint32_t pads = IOC_PAD_PAD_CTL_OD_MASK | IOC_PAD_PAD_CTL_PE_MASK | IOC_PAD_PAD_CTL_PS_MASK;
    HPM_IOC->PAD[p->scl_pad].PAD_CTL = pads;
    HPM_IOC->PAD[p->sda_pad].PAD_CTL = pads;
    HPM_IOC->PAD[p->scl_pad].FUNC_CTL = gpio ? 0 : p->scl_func | IOC_PAD_FUNC_CTL_LOOP_BACK_MASK;
    HPM_IOC->PAD[p->sda_pad].FUNC_CTL = gpio ? 0 : p->sda_func | IOC_PAD_FUNC_CTL_LOOP_BACK_MASK;
}
uint8_t bridge_board_lines(uint8_t port)
{
    const bridge_board_port_t *p = &bridge_board_ports[port];
    return (gpio_read_pin(HPM_GPIO0, GPIO_DI_GPIOC, p->scl_pin) ? 1U : 0U) |
           (gpio_read_pin(HPM_GPIO0, GPIO_DI_GPIOC, p->sda_pin) ? 2U : 0U);
}
