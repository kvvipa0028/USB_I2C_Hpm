/*
 * usb_i2c USB descriptors: one vendor interface (0xFF), bulk OUT 0x01,
 * bulk IN 0x81 (512 bytes at high speed, 64 at full speed), WinUSB
 * auto-binding via BOS + MS OS 2.0. Copied from firmware/usb_speed.
 */

#include <stdint.h>

#include "usbd_core.h"
#include "hpm_romapi.h"
#include "usb_i2c.h"

#define USBD_WINUSB_VENDOR_CODE 0x17

/* pid.codes "Test VID/PID" — for private in-house testing ONLY.
 * MUST NEVER SHIP: replace with the product VID:PID per DEV_PLAN D10. */
#define USBD_VID_SPEED 0x1209
#define USBD_PID_SPEED 0x0001

#define USB_CONFIG_SIZE (9 + 9 + 7 + 7)
#define INTF_NUM        1

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
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_IN, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_OUT, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
};

static const uint8_t config_descriptor_fs[] = {
    USB_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_IN, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_OUT, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
};

static const uint8_t device_quality_descriptor[] = {
    USB_DEVICE_QUALIFIER_DESCRIPTOR_INIT(USB_2_0, 0x00, 0x00, 0x00, 0x01),
};

static const uint8_t other_speed_config_descriptor_hs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_IN, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_OUT, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_FS, 0x00),
};

static const uint8_t other_speed_config_descriptor_fs[] = {
    USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(USB_CONFIG_SIZE, INTF_NUM, 0x01, USB_CONFIG_BUS_POWERED, USBD_MAX_POWER),
    USB_INTERFACE_DESCRIPTOR_INIT(0x00, 0x00, 0x02, 0xff, 0xff, 0x00, 0x04),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_IN, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
    USB_ENDPOINT_DESCRIPTOR_INIT(USB_I2C_EP_OUT, USB_ENDPOINT_TYPE_BULK, USB_BULK_EP_MPS_HS, 0x00),
};

static char serial_str[33]; /* chip UID as hex, filled by usb_i2c_fill_serial() */

static const char *string_descriptors[] = {
    (const char[]){ 0x09, 0x04 }, /* Langid */
    "i2c-analyzer dev",           /* Manufacturer */
    "usb_i2c",                    /* Product */
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

const struct usb_descriptor usb_i2c_descriptor = {
    .device_descriptor_callback = device_descriptor_callback,
    .config_descriptor_callback = config_descriptor_callback,
    .device_quality_descriptor_callback = device_quality_descriptor_callback,
    .other_speed_descriptor_callback = other_speed_config_descriptor_callback,
    .string_descriptor_callback = string_descriptor_callback,
    .msosv2_descriptor = &msosv2_desc,
    .bos_descriptor = &bos_desc,
};

void usb_i2c_fill_serial(void)
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
