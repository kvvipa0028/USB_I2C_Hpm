#ifndef TEST_USBD_CORE_H
#define TEST_USBD_CORE_H
#include <stdbool.h>
#include <stdint.h>
#define USB_NOCACHE_RAM_SECTION
#define USB_MEM_ALIGNX
#define USB_SPEED_HIGH 3U
enum { USBD_EVENT_RESET, USBD_EVENT_DISCONNECTED, USBD_EVENT_CONFIGURED };
struct usb_descriptor { unsigned unused; };
struct usbd_interface { unsigned unused; };
struct usbd_endpoint { uint8_t ep_addr; void (*ep_cb)(uint8_t,uint8_t,uint32_t); };
void usbd_desc_register(uint8_t,const struct usb_descriptor *);
void usbd_add_interface(uint8_t,struct usbd_interface *);
void usbd_add_endpoint(uint8_t,struct usbd_endpoint *);
void usbd_initialize(uint8_t,uint32_t,void (*)(uint8_t,uint8_t));
uint8_t usbd_get_port_speed(uint8_t);
int usbd_ep_start_read(uint8_t,uint8_t,uint8_t *,uint32_t);
int usbd_ep_start_write(uint8_t,uint8_t,const uint8_t *,uint32_t);
#endif
