/*
 * Minimal bring-up app for the I2C bus analyzer firmware on HPM6300EVK.
 * Prints a banner on the console UART and blinks the on-board LED.
 */

#include <stdio.h>
#include "board.h"
#include "hpm_debug_console.h"

#define LED_FLASH_PERIOD_IN_MS 300

int main(void)
{
    board_init();
    board_init_led_pins();

    board_timer_create(LED_FLASH_PERIOD_IN_MS, board_led_toggle);

    printf("i2c-analyzer firmware: hello world\n");
    while (1) {
    }
    return 0;
}
