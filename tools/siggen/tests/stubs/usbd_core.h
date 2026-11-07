#ifndef SIGGEN_TEST_USBD_CORE_H
#define SIGGEN_TEST_USBD_CORE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#define USB_2_0 0
#define USBD_VID 0
#define USBD_PID 0
#define USBD_MAX_POWER 0
#define USB_CONFIG_BUS_POWERED 0
#define USB_BULK_EP_MPS_HS 512
#define USB_BULK_EP_MPS_FS 64
#define USB_SPEED_HIGH 1
#define USB_DEVICE_DESCRIPTOR_INIT(...) 0
#define USB_CONFIG_DESCRIPTOR_INIT(...) 0
#define USB_DEVICE_QUALIFIER_DESCRIPTOR_INIT(...) 0
#define USB_OTHER_SPEED_CONFIG_DESCRIPTOR_INIT(...) 0
#define USB_NOCACHE_RAM_SECTION
#define USB_MEM_ALIGNX
#define USBD_EVENT_CONFIGURED 1
#define USBD_EVENT_RESET 2
#define USBD_EVENT_DISCONNECTED 3
struct usb_descriptor {
    const uint8_t *(*device_descriptor_callback)(uint8_t);
    const uint8_t *(*config_descriptor_callback)(uint8_t);
    const uint8_t *(*device_quality_descriptor_callback)(uint8_t);
    const uint8_t *(*other_speed_descriptor_callback)(uint8_t);
    const char *(*string_descriptor_callback)(uint8_t, uint8_t);
};
struct usbd_endpoint { uint8_t ep_addr; void (*ep_cb)(uint8_t, uint8_t, uint32_t); };
struct usbd_interface { unsigned unused; };
void usbd_desc_register(uint8_t, const struct usb_descriptor *);
void usbd_add_interface(uint8_t, struct usbd_interface *);
void usbd_add_endpoint(uint8_t, struct usbd_endpoint *);
void usbd_initialize(uint8_t, uint32_t, void (*)(uint8_t, uint8_t));
uint16_t usbd_get_ep_mps(uint8_t, uint8_t);
int usbd_ep_start_read(uint8_t, uint8_t, uint8_t *, uint32_t);
int usbd_ep_start_write(uint8_t, uint8_t, const uint8_t *, uint32_t);
#endif
