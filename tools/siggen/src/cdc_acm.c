/*
 * CDC ACM backend for sig5301 (adapted from the SDK's cdc_acm_vcom sample).
 * RX bytes land in a ring buffer for the line-based command parser;
 * TX goes through cdc_write() (busy-waits on the endpoint).
 */

#include "usbd_core.h"
#include "usbd_cdc_acm.h"
#include "cdc_acm.h"

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

static const uint8_t *string_descriptor_callback(uint8_t speed, uint8_t index)
{
    (void)speed;
    if (index >= sizeof(string_descriptors) / sizeof(string_descriptors[0])) {
        return NULL;
    }
    return (const uint8_t *)string_descriptors[index];
}

const struct usb_descriptor cdc_descriptor = {
    .device_descriptor_callback = device_descriptor_callback,
    .config_descriptor_callback = config_descriptor_callback,
    .device_quality_descriptor_callback = device_quality_descriptor_callback,
    .other_speed_descriptor_callback = other_speed_config_descriptor_callback,
    .string_descriptor_callback = string_descriptor_callback,
};

USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX uint8_t read_buffer[512];
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX uint8_t write_buffer[2048];
volatile bool cdc_dtr;
static volatile bool ep_tx_busy;

/* RX ring buffer for the line parser */
#define RX_RING_SIZE 1024U
static volatile uint8_t rx_ring[RX_RING_SIZE];
static volatile uint16_t rx_head, rx_tail;

static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    switch (event) {
    case USBD_EVENT_CONFIGURED:
        usbd_ep_start_read(busid, CDC_OUT_EP, read_buffer, usbd_get_ep_mps(busid, CDC_OUT_EP));
        break;
    default:
        break;
    }
}

static void usbd_cdc_acm_bulk_out(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    for (uint32_t i = 0; i < nbytes; i++) {
        uint16_t next = (uint16_t)((rx_head + 1) % RX_RING_SIZE);
        if (next != rx_tail) {
            rx_ring[rx_head] = read_buffer[i];
            rx_head = next;
        }
    }
    usbd_ep_start_read(busid, ep, read_buffer, usbd_get_ep_mps(busid, ep));
}

static void usbd_cdc_acm_bulk_in(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    if ((nbytes % usbd_get_ep_mps(busid, ep)) == 0 && nbytes != 0) {
        usbd_ep_start_write(busid, ep, NULL, 0); /* zlp */
    } else {
        ep_tx_busy = false;
    }
}

static struct usbd_endpoint cdc_out_ep = { .ep_addr = CDC_OUT_EP, .ep_cb = usbd_cdc_acm_bulk_out };
static struct usbd_endpoint cdc_in_ep = { .ep_addr = CDC_IN_EP, .ep_cb = usbd_cdc_acm_bulk_in };
static struct usbd_interface intf0;
static struct usbd_interface intf1;

void cdc_acm_init(uint8_t busid, uint32_t reg_base)
{
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
}

int cdc_getchar(void)
{
    if (rx_tail == rx_head) {
        return -1;
    }
    uint8_t c = rx_ring[rx_tail];
    rx_tail = (uint16_t)((rx_tail + 1) % RX_RING_SIZE);
    return c;
}

void cdc_write(const char *buf, uint32_t len)
{
    if (len == 0 || !cdc_dtr) {
        return;
    }
    while (len > sizeof(write_buffer)) {
        cdc_write(buf, sizeof(write_buffer));
        buf += sizeof(write_buffer);
        len -= sizeof(write_buffer);
    }
    ep_tx_busy = true;
    memcpy(write_buffer, buf, len);
    usbd_ep_start_write(0, CDC_IN_EP, write_buffer, len);
    while (ep_tx_busy) {
    }
}
