#ifndef USB_I2C_H
#define USB_I2C_H

#include <stdint.h>

#define USB_I2C_EP_OUT 0x01
#define USB_I2C_EP_IN  0x81

struct usb_descriptor;
extern const struct usb_descriptor usb_i2c_descriptor;   /* usb_desc.c */
void usb_i2c_fill_serial(void);                          /* usb_desc.c */

void usb_i2c_init(uint8_t busid, uint32_t reg_base);
void usb_i2c_task(void);   /* call from the main loop */

#endif
