/* One bounded request/reply slot. RX is rearmed only after IN completion.
 * USB reset changes the epoch, suppresses stale replies and never replays I2C.
 */
#include <string.h>
#include "board.h"
#include "usbd_core.h"
#include "usb_config.h"
#include "usb_i2c.h"
#include "bridge_protocol.h"

#define BUF_BYTES 512U
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t rx_buf[BUF_BYTES];
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t tx_buf[BUF_BYTES];
static uint8_t s_busid;
static volatile bool configured, rx_ready, rx_armed, tx_done;
static volatile uint32_t rx_len, epoch;
static uint32_t task_epoch;
static volatile uint32_t tx_epoch;
static bool active;
static volatile bool tx_busy;
static bridge_protocol_t protocol;

static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    (void)busid;
    switch (event) {
    case USBD_EVENT_RESET:
    case USBD_EVENT_DISCONNECTED:
        configured = false; rx_ready = false; rx_armed = false; ++epoch;
        break;
    case USBD_EVENT_CONFIGURED:
        configured = true; rx_ready = false; rx_armed = false; ++epoch;
        /* Main loop first aborts the preceding epoch, then arms RX. */
        break;
    default: break;
    }
}
static void ep_out_cb(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid; (void)ep;
    if (configured && rx_armed) {
        rx_len = nbytes; rx_armed = false; rx_ready = true;
    }
}
static void ep_in_cb(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid; (void)ep; (void)nbytes;
    /* HPM reset clears ENDPTCOMPLETE and flushes QHD/QTD before reconfigure.
     * Disconnect can still share an IRQ with an old completion: ignore it. */
    if (configured && tx_busy && tx_epoch == epoch) tx_done = true;
}
static struct usbd_endpoint out_ep = { .ep_addr = USB_I2C_EP_OUT, .ep_cb = ep_out_cb };
static struct usbd_endpoint in_ep = { .ep_addr = USB_I2C_EP_IN, .ep_cb = ep_in_cb };
static struct usbd_interface intf;

void usb_i2c_task(void)
{
    if (task_epoch != epoch) {
        bridge_ports_abort();
        active = false; tx_busy = false; tx_done = false;
        task_epoch = epoch;
    }
    bridge_ports_poll();
    if (!configured) return;
    if (tx_busy) {
        if (!tx_done) return;
        tx_done = false; tx_busy = false; active = false;
    }
    if (rx_ready && !active) {
        rx_ready = false; active = true;
        uint16_t mps = usbd_get_port_speed(s_busid) == USB_SPEED_HIGH ? 512 : 64;
        bridge_protocol_start(&protocol, rx_buf, rx_len, mps, tx_buf);
    }
    if (active && bridge_protocol_poll(&protocol)) {
        /* Exclude USB callbacks only: slave IRQs stay serviceable while the
         * endpoint driver builds/primes its descriptor. */
        intc_m_disable_irq(CONFIG_HPM_USBD_IRQn);
        if (configured && task_epoch == epoch) {
            tx_epoch = epoch; tx_done = false;
            tx_busy = true;
            if (usbd_ep_start_write(s_busid, USB_I2C_EP_IN, tx_buf, protocol.response_len) != 0) {
                configured = false; ++epoch;
            }
        }
        intc_m_enable_irq(CONFIG_HPM_USBD_IRQn);
        return;
    }
    if (!active && !rx_armed) {
        intc_m_disable_irq(CONFIG_HPM_USBD_IRQn);
        if (configured && task_epoch == epoch) {
            rx_armed = true;
            if (usbd_ep_start_read(s_busid, USB_I2C_EP_OUT, rx_buf, BUF_BYTES) != 0) {
                configured = false; rx_armed = false; ++epoch;
            }
        }
        intc_m_enable_irq(CONFIG_HPM_USBD_IRQn);
    }
}
void usb_i2c_init(uint8_t busid, uint32_t reg_base)
{
    s_busid = busid;
    usb_i2c_fill_serial();
    usbd_desc_register(busid, &usb_i2c_descriptor);
    usbd_add_interface(busid, &intf);
    usbd_add_endpoint(busid, &out_ep);
    usbd_add_endpoint(busid, &in_ep);
    usbd_initialize(busid, reg_base, usbd_event_handler);
}
