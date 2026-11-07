/*
 * CDC ACM backend for sig5301 (adapted from the SDK's cdc_acm_vcom sample).
 * RX bytes land in a ring buffer for the line-based command parser;
 * TX is single-flight and non-blocking; a stalled host cannot stop deadlines.
 */

#include "usbd_core.h"
#include "usbd_cdc_acm.h"
#include "cdc_acm.h"
#include "cdc_buffer.h"
#include "siggen_board.h"

/*!< endpoint address */
#define CDC_IN_EP  0x81
#define CDC_OUT_EP 0x01
#define CDC_INT_EP 0x83

/*!< config descriptor size */
#define USB_CONFIG_SIZE (9 + CDC_ACM_DESCRIPTOR_LEN)

static const uint8_t device_descriptor[] = {
    USB_DEVICE_DESCRIPTOR_INIT(USB_2_0, 0xEF, 0x02, 0x01, USBD_VID, USBD_PID, 0x0100, 0x01)
};

static const uint8_t config_descriptor_hs[] = {
    USB_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, 0x02, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    CDC_ACM_DESCRIPTOR_INIT(0x00, CDC_INT_EP, CDC_OUT_EP, CDC_IN_EP, USB_BULK_EP_MPS_HS, 0x02),
};

static const uint8_t config_descriptor_fs[] = {
    USB_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, 0x02, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    CDC_ACM_DESCRIPTOR_INIT(0x00, CDC_INT_EP, CDC_OUT_EP, CDC_IN_EP, USB_BULK_EP_MPS_FS, 0x02),
};

static const uint8_t device_quality_descriptor[] = {
    USB_DEVICE_QUALIFIER_DESCRIPTOR_INIT(USB_2_0, 0xEF, 0x02, 0x01, 0x01),
};

static const uint8_t other_speed_config_descriptor_hs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, 0x02, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    CDC_ACM_DESCRIPTOR_INIT(0x00, CDC_INT_EP, CDC_OUT_EP, CDC_IN_EP, USB_BULK_EP_MPS_FS, 0x02),
};

static const uint8_t other_speed_config_descriptor_fs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, 0x02, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    CDC_ACM_DESCRIPTOR_INIT(0x00, CDC_INT_EP, CDC_OUT_EP, CDC_IN_EP, USB_BULK_EP_MPS_HS, 0x02),
};

static const char *string_descriptors[] = {
    (const char[]){ 0x09, 0x04 }, /* Langid */
    "i2c-analyzer dev",           /* Manufacturer */
    "sig5301",                    /* Product */
    "sig5301-0001",               /* Serial Number */
};

static const uint8_t *device_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_descriptor;
}

static const uint8_t *config_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return speed == USB_SPEED_HIGH ? config_descriptor_hs : config_descriptor_fs;
}

static const uint8_t *device_quality_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_quality_descriptor;
}

static const uint8_t *other_speed_config_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return speed == USB_SPEED_HIGH ? other_speed_config_descriptor_hs : other_speed_config_descriptor_fs;
}

static const char *string_descriptor_callback(uint8_t speed, uint8_t index)
{
    (void)speed;
    if (index >= sizeof(string_descriptors) / sizeof(string_descriptors[0])) {
        return NULL;
    }
    return string_descriptors[index];
}

const struct usb_descriptor cdc_descriptor = {
    .device_descriptor_callback = device_descriptor_callback,
    .config_descriptor_callback = config_descriptor_callback,
    .device_quality_descriptor_callback = device_quality_descriptor_callback,
    .other_speed_descriptor_callback = other_speed_config_descriptor_callback,
    .string_descriptor_callback = string_descriptor_callback,
};

USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX uint8_t read_buffer[512];
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX uint8_t write_buffer[SIGGEN_CDC_TX_CAP];
volatile bool cdc_dtr;
static bool configured, ep_tx_busy;
/* Kept separate from logical connection state: a DTR drop does not cancel an
 * IN DMA transfer. Do not reuse its buffer until completion or USB reset. */
static siggen_cdc_buffer_t buffer;

static void lost(void)
{
    siggen_cdc_buffer_connected(&buffer, false);
}
static void arm_read(uint8_t busid)
{
    uint16_t mps = usbd_get_ep_mps(busid, CDC_OUT_EP);
    if (mps == 0 || mps > sizeof(read_buffer) ||
        usbd_ep_start_read(busid, CDC_OUT_EP, read_buffer, mps) != 0) {
        configured = false;
        lost();
    }
}
static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    switch (event) {
    case USBD_EVENT_CONFIGURED:
        configured = true;
        siggen_cdc_buffer_connected(&buffer, cdc_dtr);
        arm_read(busid);
        break;
    case USBD_EVENT_RESET:
    case USBD_EVENT_DISCONNECTED:
        configured = false;
        cdc_dtr = false;
        ep_tx_busy = false;
        lost();
        break;
    default:
        break;
    }
}
static void usbd_cdc_acm_bulk_out(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)ep;
    if (!configured) return;
    if (nbytes > sizeof(read_buffer)) { configured = false; lost(); return; }
    siggen_cdc_buffer_rx(&buffer, read_buffer, nbytes);
    arm_read(busid);
}
static void usbd_cdc_acm_bulk_in(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    if (!configured || !ep_tx_busy) return;
    uint16_t mps = usbd_get_ep_mps(busid, ep);
    if (mps != 0 && nbytes != 0 && nbytes % mps == 0) {
        if (usbd_ep_start_write(busid, ep, NULL, 0) == 0) return;
        configured = false;
        lost();
    }
    ep_tx_busy = false;
    siggen_cdc_buffer_tx_done(&buffer);
}

static struct usbd_endpoint cdc_out_ep = { .ep_addr = CDC_OUT_EP, .ep_cb = usbd_cdc_acm_bulk_out };
static struct usbd_endpoint cdc_in_ep = { .ep_addr = CDC_IN_EP, .ep_cb = usbd_cdc_acm_bulk_in };
static struct usbd_interface intf0;
static struct usbd_interface intf1;

void cdc_acm_init(uint8_t busid, uint32_t reg_base)
{
    siggen_cdc_buffer_init(&buffer);
    usbd_desc_register(busid, &cdc_descriptor);
    usbd_add_interface(busid, usbd_cdc_acm_init_intf(busid, &intf0));
    usbd_add_interface(busid, usbd_cdc_acm_init_intf(busid, &intf1));
    usbd_add_endpoint(busid, &cdc_out_ep);
    usbd_add_endpoint(busid, &cdc_in_ep);
    usbd_initialize(busid, reg_base, usbd_event_handler);
}

void usbd_cdc_acm_set_dtr(uint8_t busid, uint8_t intf, bool dtr)
{
    (void)busid;
    (void)intf;
    cdc_dtr = dtr;
    siggen_cdc_buffer_connected(&buffer, configured && dtr);
}
int cdc_getchar(void)
{
    uint32_t irq = siggen_board_lock();
    int c = siggen_cdc_buffer_getchar(&buffer);
    siggen_board_unlock(irq);
    return c;
}
bool cdc_can_write(void)
{
    uint32_t irq = siggen_board_lock();
    bool ready = !ep_tx_busy && siggen_cdc_buffer_can_write(&buffer);
    siggen_board_unlock(irq);
    return ready;
}
bool cdc_take_session_lost(void)
{
    uint32_t irq = siggen_board_lock();
    bool result = siggen_cdc_buffer_take_session_lost(&buffer);
    siggen_board_unlock(irq);
    return result;
}
void cdc_stats(uint32_t *rx_overflows, uint32_t *tx_rejected)
{
    uint32_t irq = siggen_board_lock();
    *rx_overflows = buffer.rxoverflow;
    *tx_rejected = buffer.txreject;
    siggen_board_unlock(irq);
}
bool cdc_write(const char *data, uint32_t len)
{
    uint32_t irq = siggen_board_lock();
    bool accepted = !ep_tx_busy && siggen_cdc_buffer_write(&buffer, (const uint8_t *)data, len);
    if (accepted) {
        memcpy(write_buffer, siggen_cdc_buffer_tx_data(&buffer), len);
        ep_tx_busy = true;
        if (usbd_ep_start_write(0, CDC_IN_EP, write_buffer, len) != 0) {
            ep_tx_busy = false;
            configured = false;
            lost();
            accepted = false;
        }
    }
    siggen_board_unlock(irq);
    return accepted;
}
