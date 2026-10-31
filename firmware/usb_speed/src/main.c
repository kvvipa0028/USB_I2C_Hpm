/*
 * usb_speed: USB HS bulk throughput test firmware for HPM6300EVK.
 * Streams a continuous u32 counter pattern on bulk IN 0x81 and sinks
 * everything the host writes to bulk OUT 0x01.
 */

#include <stdio.h>
#include "board.h"
#include "usb_config.h"

extern void usb_speed_init(uint8_t busid, uint32_t reg_base);
extern void usb_speed_stats_task(void);

int main(void)
{
    board_init();
    board_init_usb((USB_Type *)CONFIG_HPM_USBD_BASE);

    intc_set_irq_priority(CONFIG_HPM_USBD_IRQn, 1);

    printf("usb_speed: bulk IN/OUT throughput test\n");

    usb_speed_init(0, CONFIG_HPM_USBD_BASE);

    while (1) {
        usb_speed_stats_task();
    }
    return 0;
}
