#include <string.h>
#include <stdio.h>
#include <inttypes.h>
#include "board.h"
#include "usb_config.h"
#include "cdc_acm.h"
#include "siggen_board.h"
#include "siggen_session.h"

/* JTAG-visible software observations, not measured electrical timing. */
siggen_peer_t g_siggen;
static siggen_line_assembler_t line;
static siggen_reply_t reply;
static char response[SIGGEN_RESPONSE_CAP];

int main(void)
{
    board_init();
    siggen_board_init(&g_siggen);
    siggen_peer_init(&g_siggen, siggen_board_hal());
    siggen_line_assembler_init(&line);
    board_init_usb((USB_Type *)CONFIG_HPM_USBD_BASE);
    intc_set_irq_priority(CONFIG_HPM_USBD_IRQn, 2);
    cdc_acm_init(0, CONFIG_HPM_USBD_BASE);
    for (;;) {
        /* Deadlines keep advancing even when the host stops reading CDC IN. */
        uint32_t irq = siggen_board_lock();
        if (cdc_take_session_lost()) {
            siggen_peer_stop(&g_siggen);
            siggen_line_assembler_init(&line);
        }
        siggen_peer_poll(&g_siggen, siggen_board_now_us());
        siggen_board_unlock(irq);
        /* Bound parsing work between hardware deadline polls. */
        for (unsigned i = 0; i < 64 && cdc_can_write(); ++i) {
            int byte = cdc_getchar();
            if (byte == -1) break;
            siggen_command_t command;
            const char *error;
            enum siggen_session_event event = siggen_session_parse(&line, byte, &command, &error);
            irq = siggen_board_lock();
            /* A loss observed after dequeuing a byte also cancels its command. */
            if (cdc_take_session_lost()) {
                siggen_peer_stop(&g_siggen);
                siggen_line_assembler_init(&line);
                event = SIG_SESSION_NONE;
                error = NULL;
            } else {
                if (event == SIG_SESSION_STOP || event == SIG_SESSION_STOP_ERROR)
                    siggen_peer_stop(&g_siggen);
                if (event == SIG_SESSION_COMMAND)
                    siggen_peer_prepare(&g_siggen, &command, siggen_board_now_us(), &reply);
            }
            siggen_board_unlock(irq);
            if (event == SIG_SESSION_COMMAND) siggen_peer_format(&reply, response, sizeof(response));
            else if (error != NULL) (void)snprintf(response, sizeof(response), "%s", error);
            if (event == SIG_SESSION_COMMAND || error != NULL) {
                if (strncmp(response, "OK running=", 11) == 0) {
                    uint32_t rx_overflows, tx_rejected;
                    cdc_stats(&rx_overflows, &tx_rejected);
                    size_t used = strlen(response) - 1U;
                    (void)snprintf(response + used, sizeof(response) - used,
                        " usb_rx_overflows=%" PRIu32 " usb_tx_rejected=%" PRIu32 "\n",
                        rx_overflows, tx_rejected);
                }
                (void)cdc_write(response, (uint32_t)strlen(response));
            }
        }
        /* No delay_ms, SDK blocking I2C call or CDC endpoint busy-wait. */
    }
}
