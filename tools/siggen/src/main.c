/*
 * siggen: I2C master test peer on an HPMicro EVK. Board-agnostic: build
 * with -DBOARD=hpm6750evkmini (or any board defining BOARD_APP_I2C_BASE).
 *
 * USB CDC command line (line based, `OK ...` / `ERR <reason>`):
 *   start   - begin issuing one transaction round every 100 ms on
 *             BOARD_APP_I2C_BASE: write {0xDE,0xAD,0x00} to 0x50, then
 *             read 2 bytes from 0x50
 *   stop    - stop the rounds
 *   status  - print running state and round count
 *
 * With no slave attached the address byte is NACKed.
 */

#include <stdio.h>
#include <string.h>
#include "board.h"
#include "hpm_i2c_drv.h"
#include "usb_config.h"
#include "cdc_acm.h"

#define SIG_I2C        BOARD_APP_I2C_BASE
#define SIG_ADDR       0x50U
#define SIG_PERIOD_MS  100U

static const uint8_t sig_wr[3] = { 0xDE, 0xAD, 0x00 };

static volatile bool g_running;
static volatile uint32_t g_rounds;

static char g_line[96];
static uint16_t g_linelen;

static void cmd_help(void)
{
    cdc_write("commands: start | stop | status\n", 32);
}

static void handle_line(const char *cmd)
{
    if (!strcmp(cmd, "start")) {
        g_running = true;
        cdc_write("OK start\n", 9);
    } else if (!strcmp(cmd, "stop")) {
        g_running = false;
        cdc_write("OK stop\n", 8);
    } else if (!strcmp(cmd, "status")) {
        char buf[64];
        int n = snprintf(buf, sizeof(buf), "OK running=%d rounds=%lu\n",
                         g_running ? 1 : 0, (unsigned long)g_rounds);
        cdc_write(buf, (uint32_t)n);
    } else if (!strcmp(cmd, "help")) {
        cmd_help();
    } else {
        cdc_write("ERR unknown command\n", 21);
    }
}

static void poll_cdc_line(void)
{
    int c;
    while ((c = cdc_getchar()) >= 0) {
        if (c == '\r' || c == '\n') {
            if (g_linelen > 0) {
                g_line[g_linelen] = '\0';
                handle_line(g_line);
                g_linelen = 0;
            }
        } else if (g_linelen < sizeof(g_line) - 1) {
            g_line[g_linelen++] = (char)c;
        }
    }
}

static void issue_round(void)
{
    uint8_t rd[2];
    /* with no slave on the bus these calls return NACK errors */
    (void)i2c_master_write(SIG_I2C, SIG_ADDR, sig_wr, sizeof(sig_wr));
    (void)i2c_master_read(SIG_I2C, SIG_ADDR, rd, sizeof(rd));
    g_rounds++;
}

int main(void)
{
    uint32_t tick = 0;

    board_init();
    board_init_usb((USB_Type *)CONFIG_HPM_USBD_BASE);
    intc_set_irq_priority(CONFIG_HPM_USBD_IRQn, 2);
    cdc_acm_init(0, CONFIG_HPM_USBD_BASE);

    board_init_i2c(SIG_I2C);

    while (1) {
        poll_cdc_line();
        if (g_running) {
            tick++;
            if (tick >= SIG_PERIOD_MS) {
                tick = 0;
                issue_round();
            }
        }
        board_delay_ms(1);
    }
    return 0;
}
