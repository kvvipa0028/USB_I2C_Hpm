/*
 * usb_speed: vendor-class bulk throughput test on CherryUSB.
 *
 * - One vendor-specific interface (0xFF), bulk OUT 0x01, bulk IN 0x81,
 *   wMaxPacketSize 512 at high speed (64 at full speed).
 * - IN streams an endless little-endian u32 counter so the host can detect
 *   drops/duplicates; two 16 KiB buffers are kept ping-ponging so the IN pipe
 *   is re-armed inside the completion callback.
 * - OUT sinks and counts bytes.
 * - WinUSB auto-binding via BOS + MS OS 2.0 descriptor set.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "usbd_core.h"
#include "hpm_romapi.h"
#include "hpm_mchtmr_drv.h"
#include "hpm_clock_drv.h"
#include "hpm_interrupt.h"

#define USBD_WINUSB_VENDOR_CODE 0x17

#define SPEED_IN_EP  0x81
#define SPEED_OUT_EP 0x01

/* pid.codes "Test VID/PID" — for private in-house testing ONLY.
 * MUST NEVER SHIP: replace with the product VID:PID per DEV_PLAN D10. */
#define USBD_VID_SPEED 0x1209
#define USBD_PID_SPEED 0x0001

#define USB_CONFIG_SIZE (9 + 9 + 7 + 7)
#define INTF_NUM        1

/* 16 KiB per ping-pong half; multiple of the 512-byte HS max packet */
#define TX_BUF_BYTES (16U * 1024U)
#define RX_BUF_BYTES (8U * 1024U)

/* MS OS 2.0 descriptor set: CompatibleID WINUSB + DeviceInterfaceGUIDs
 * registry property ({68A0B528-096A-4023-AE2D-EFB583CA10DF}) so Windows 10/11
 * binds WinUSB to the whole device without a driver install. */
static const uint8_t USBD_WinUSBDescriptorSetDescriptor[] = {
    USB_MSOSV2_COMP_ID_SET_HEADER_DESCRIPTOR_INIT(WINUSB_DESCRIPTOR_SET_HEADER_SIZE + WINUSB_FEATURE_COMPATIBLE_ID_SIZE + 132),
    WBVAL(WINUSB_FEATURE_COMPATIBLE_ID_SIZE),  /* wLength */
    WBVAL(WINUSB_FEATURE_COMPATIBLE_ID_TYPE),  /* wDescriptorType */
    'W', 'I', 'N', 'U', 'S', 'B', 0, 0,        /* CompatibleId */
    0, 0, 0, 0, 0, 0, 0, 0,                    /* SubCompatibleId */
    WBVAL(132),                                /* wLength */
    WBVAL(WINUSB_FEATURE_REG_PROPERTY_TYPE),   /* wDescriptorType */
    /* MS OS 2.0 spec requires DeviceInterfaceGUIDs as REG_MULTI_SZ; the
     * SINGLE-descriptor macro in CherryUSB's usb_def.h uses REG_SZ instead
     * and is not followed here (its MULTI variant and daplink do use it). */
    WBVAL(WINUSB_PROP_DATA_TYPE_REG_MULTI_SZ), /* wPropertyDataType */
    WBVAL(42),                                 /* wPropertyNameLength: "DeviceInterfaceGUIDs" */
    'D', 0, 'e', 0, 'v', 0, 'i', 0, 'c', 0, 'e', 0,
    'I', 0, 'n', 0, 't', 0, 'e', 0, 'r', 0, 'f', 0, 'a', 0, 'c', 0, 'e', 0,
    'G', 0, 'U', 0, 'I', 0, 'D', 0, 's', 0, 0, 0,
    WBVAL(80),                                 /* wPropertyDataLength */
    '{', 0,
    '6', 0, '8', 0, 'A', 0, '0', 0, 'B', 0, '5', 0, '2', 0, '8', 0, '-', 0,
    '0', 0, '9', 0, '6', 0, 'A', 0, '-', 0,
    '4', 0, '0', 0, '2', 0, '3', 0, '-', 0,
    'A', 0, 'E', 0, '2', 0, 'D', 0, '-', 0,
    'E', 0, 'F', 0, 'B', 0, '5', 0, '8', 0, '3', 0, 'C', 0, 'A', 0,
    '1', 0, '0', 0, 'D', 0, 'F', 0,
    '}', 0, 0, 0, 0, 0
};

static const uint8_t USBD_BinaryObjectStoreDescriptor[] = {
    USB_BOS_HEADER_DESCRIPTOR_INIT(5 + USB_BOS_CAP_PLATFORM_WINUSB_DESCRIPTOR_LEN, 1),
    USB_BOS_CAP_PLATFORM_WINUSB_DESCRIPTOR_INIT(USBD_WINUSB_VENDOR_CODE, sizeof(USBD_WinUSBDescriptorSetDescriptor)),
};

const struct usb_msosv2_descriptor msosv2_desc = {
    .vendor_code = USBD_WINUSB_VENDOR_CODE,
    .compat_id = USBD_WinUSBDescriptorSetDescriptor,
    .compat_id_len = sizeof(USBD_WinUSBDescriptorSetDescriptor),
};

const struct usb_bos_descriptor bos_desc = {
    .string = USBD_BinaryObjectStoreDescriptor,
    .string_len = sizeof(USBD_BinaryObjectStoreDescriptor)
};

static const uint8_t device_descriptor[] = {
    USB_DEVICE_DESCRIPTOR_INIT(USB_2_1, 0x00, 0x00, 0x00, USBD_VID_SPEED, USBD_PID_SPEED, 0x0100, 0x01),
};

static const uint8_t config_descriptor_hs[] = {
    USB_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_IN_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_OUT_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
};

static const uint8_t config_descriptor_fs[] = {
    USB_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_IN_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_OUT_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
};

static const uint8_t device_quality_descriptor[] = {
    USB_DEVICE_QUALIFIER_DESCRIPTOR_INIT(USB_2_0, 0x00, 0x00, 0x00, 0x01),
};

static const uint8_t other_speed_config_descriptor_hs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_IN_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_OUT_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
};

static const uint8_t other_speed_config_descriptor_fs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_IN_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(SPEED_OUT_EP, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
};

static char serial_str[33]; /* chip UID as hex, filled by fill_serial() */

static const char *string_descriptors[] = {
    (const char[]){ 0x09, 0x04 }, /* Langid */
    "i2c-analyzer dev",           /* Manufacturer */
    "usb_speed",                  /* Product */
    serial_str,                   /* Serial Number */
};

static const uint8_t *device_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_descriptor;
}

static const uint8_t *config_descriptor_callback(uint8_t speed)
{
    if (speed == USB_SPEED_HIGH) {
        return config_descriptor_hs;
    } else if (speed == USB_SPEED_FULL) {
        return config_descriptor_fs;
    } else {
        return NULL;
    }
}

static const uint8_t *device_quality_descriptor_callback(uint8_t speed)
{
    (void)speed;
    return device_quality_descriptor;
}

static const uint8_t *other_speed_config_descriptor_callback(uint8_t speed)
{
    if (speed == USB_SPEED_HIGH) {
        return other_speed_config_descriptor_hs;
    } else if (speed == USB_SPEED_FULL) {
        return other_speed_config_descriptor_fs;
    } else {
        return NULL;
    }
}

static const char *string_descriptor_callback(uint8_t speed, uint8_t index)
{
    (void)speed;
    if (index >= (sizeof(string_descriptors) / sizeof(char *))) {
        return NULL;
    }
    return string_descriptors[index];
}

const struct usb_descriptor usb_speed_descriptor = {
    .device_descriptor_callback = device_descriptor_callback,
    .config_descriptor_callback = config_descriptor_callback,
    .device_quality_descriptor_callback = device_quality_descriptor_callback,
    .other_speed_descriptor_callback = other_speed_config_descriptor_callback,
    .string_descriptor_callback = string_descriptor_callback,
    .msosv2_descriptor = &msosv2_desc,
    .bos_descriptor = &bos_desc,
};

/* ---------------- streaming engine ---------------- */

USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t tx_buf[2][TX_BUF_BYTES];
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t rx_buf[RX_BUF_BYTES];

static volatile uint8_t  configured;
static volatile uint8_t  tx_inflight; /* buffer index currently on the wire */
static uint32_t          tx_word;     /* next u32 pattern word */
static volatile uint64_t rx_bytes;    /* total bytes sunk on OUT */

/* device-side 1-second-window stats: 32-bit per-window counters are
 * snapshotted and reset with interrupts disabled so nothing is read torn */
static uint32_t          mchtmr_hz;
static volatile uint32_t in_bytes_win;
static volatile uint32_t out_bytes_win;
static volatile uint32_t max_in_gap_us_win;
static uint32_t          prev_in_done;
static uint8_t           have_prev_in;

static void tx_fill(uint8_t *buf)
{
    uint32_t *w = (uint32_t *)buf;
    uint32_t i;
    for (i = 0; i < TX_BUF_BYTES / 4; i++) {
        w[i] = tx_word++;
    }
}

static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    switch (event) {
    case USBD_EVENT_RESET:
    case USBD_EVENT_DISCONNECTED:
        configured = 0;
        break;
    case USBD_EVENT_CONFIGURED:
        configured = 1;
        tx_inflight = 0;
        tx_word = 0;
        rx_bytes = 0;
        have_prev_in = 0;
        in_bytes_win = 0;
        out_bytes_win = 0;
        max_in_gap_us_win = 0;
        tx_fill(tx_buf[0]);
        tx_fill(tx_buf[1]);
        usbd_ep_start_write(busid, SPEED_IN_EP, tx_buf[0], TX_BUF_BYTES);
        usbd_ep_start_read(busid, SPEED_OUT_EP, rx_buf, RX_BUF_BYTES);
        break;
    default:
        break;
    }
}

static void usbd_speed_in(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    uint8_t done;
    uint32_t now;
    (void)ep;
    if (!configured) {
        return;
    }
    /* how long the IN pipe sat idle between completions (host-side stall) */
    now = (uint32_t)mchtmr_get_count(HPM_MCHTMR);
    if (have_prev_in) {
        uint32_t gap_us =
            (uint32_t)(((uint64_t)(now - prev_in_done) * 1000000ULL) / mchtmr_hz);
        if (gap_us > max_in_gap_us_win) {
            max_in_gap_us_win = gap_us;
        }
    }
    have_prev_in = 1;
    prev_in_done = now;
    in_bytes_win += nbytes;
    /* re-arm the other (already filled) buffer first to minimise the gap,
     * then refill the one that just completed */
    done = tx_inflight;
    tx_inflight ^= 1;
    usbd_ep_start_write(busid, SPEED_IN_EP, tx_buf[tx_inflight], TX_BUF_BYTES);
    tx_fill(tx_buf[done]);
}

static void usbd_speed_out(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    rx_bytes += nbytes;
    out_bytes_win += nbytes;
    if (configured) {
        usbd_ep_start_read(busid, ep, rx_buf, RX_BUF_BYTES);
    }
}

struct usbd_endpoint speed_out_ep = {
    .ep_addr = SPEED_OUT_EP,
    .ep_cb = usbd_speed_out
};

struct usbd_endpoint speed_in_ep = {
    .ep_addr = SPEED_IN_EP,
    .ep_cb = usbd_speed_in
};

struct usbd_interface speed_intf;

static void fill_serial(void)
{
    /* OTP chip UID: 4 words at OTP indices 8..11 -> 32 hex chars */
    static const char hex[] = "0123456789abcdef";
    uint8_t uid[16];
    uint32_t i, j, k = 0;
    for (i = 0; i < 4; i++) {
        uint32_t w = ROM_API_TABLE_ROOT->otp_driver_if->read_from_shadow(8 + i);
        uid[k++] = (uint8_t)(w >> 24);
        uid[k++] = (uint8_t)(w >> 16);
        uid[k++] = (uint8_t)(w >> 8);
        uid[k++] = (uint8_t)w;
    }
    for (i = 0, j = 0; i < 16; i++) {
        serial_str[j++] = hex[uid[i] >> 4];
        serial_str[j++] = hex[uid[i] & 0x0F];
    }
    serial_str[32] = '\0';
}

/* Print per-second device-side stats from the main loop (never from ISR).
 * Snapshot-and-reset the 32-bit window counters with interrupts disabled. */
void usb_speed_stats_task(void)
{
    static uint32_t prev;
    static uint32_t sec;
    uint32_t now = (uint32_t)mchtmr_get_count(HPM_MCHTMR);
    uint32_t ib, ob, g;

    if (mchtmr_hz == 0 || (now - prev) < mchtmr_hz) {
        return;
    }
    prev = now;

    {
        uint32_t irq_state = disable_global_irq(CSR_MSTATUS_MIE_MASK);
        ib = in_bytes_win;
        ob = out_bytes_win;
        g = max_in_gap_us_win;
        in_bytes_win = 0;
        out_bytes_win = 0;
        max_in_gap_us_win = 0;
        set_csr(CSR_MSTATUS, irq_state);
    }

    sec++;
    {
        /* MiB/s with one decimal, integer math (SDK printf may lack %f) */
        uint32_t in_x10 = (uint32_t)((ib * 10ULL) >> 20);
        uint32_t out_x10 = (uint32_t)((ob * 10ULL) >> 20);
        printf("t=%lus in=%lu.%lu out=%lu.%lu max_in_gap_us=%lu\n",
               (unsigned long)sec,
               (unsigned long)(in_x10 / 10), (unsigned long)(in_x10 % 10),
               (unsigned long)(out_x10 / 10), (unsigned long)(out_x10 % 10),
               (unsigned long)g);
    }
}

void usb_speed_init(uint8_t busid, uint32_t reg_base)
{
    mchtmr_hz = clock_get_frequency(clock_mchtmr0);
    fill_serial();
    usbd_desc_register(busid, &usb_speed_descriptor);
    usbd_add_interface(busid, &speed_intf);
    usbd_add_endpoint(busid, &speed_out_ep);
    usbd_add_endpoint(busid, &speed_in_ep);
    usbd_initialize(busid, reg_base, usbd_event_handler);
}
