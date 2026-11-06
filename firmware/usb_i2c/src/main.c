/*
 * usb_i2c: USB-to-I2C bridge prototype for HPM6300EVK.
 * Two independent master/slave ports; protocol is in bridge_protocol.c.
 */

#include "board.h"
#include "usb_config.h"
#include "usb_i2c.h"
#include "i2c_port.h"

int main(void)
{
    board_init();
    bridge_ports_init();
    board_init_usb((USB_Type *)CONFIG_HPM_USBD_BASE);
    intc_set_irq_priority(CONFIG_HPM_USBD_IRQn, 1);

    usb_i2c_init(0, CONFIG_HPM_USBD_BASE);

    while (1) {
        usb_i2c_task();
    }
    return 0;
}
