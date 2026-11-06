#ifndef SIGGEN_TEST_USBD_CDC_ACM_H
#define SIGGEN_TEST_USBD_CDC_ACM_H
#include "usbd_core.h"
#define CDC_ACM_DESCRIPTOR_LEN 1
#define CDC_ACM_DESCRIPTOR_INIT(...) 0
struct usbd_interface *usbd_cdc_acm_init_intf(uint8_t, struct usbd_interface *);
#endif
