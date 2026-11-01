/*
 * usb_i2c: USB-to-I2C bridge prototype for HPM6300EVK.
 * The PC sends commands on bulk OUT 0x01; the firmware runs them on I2C0
 * (master, 100 kHz) and replies on bulk IN 0x81. Protocol: usb_i2c.c.
 */

#include "board.h"
#include "usb_config.h"
#include "usb_i2c.h"

int main(void)
{
    board_init();
    board_init_i2c(BOARD_APP_I2C_BASE);   /* I2C0 master, 100 kHz */
    board_init_usb((USB_Type *)CONFIG_HPM_USBD_BASE);
    intc_set_irq_priority(CONFIG_HPM_USBD_IRQn, 1);

    usb_i2c_init(0, CONFIG_HPM_USBD_BASE);

    while (1) {
        usb_i2c_task();
    }
    return 0;
}
