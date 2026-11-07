#include "usb_i2c.h"
#include "usbd_core.h"
#include "bridge_protocol.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
const struct usb_descriptor usb_i2c_descriptor={0};
static void (*event_handler)(uint8_t,uint8_t);
static struct usbd_endpoint *in_ep,*out_ep;
static uint8_t *rx;
static unsigned reads,writes,aborts,commits;
static bool ready=true,reset_in_command,fail_write;
void usb_i2c_fill_serial(void) {}
void usbd_desc_register(uint8_t b,const struct usb_descriptor *d){(void)b;(void)d;}
void usbd_add_interface(uint8_t b,struct usbd_interface *i){(void)b;(void)i;}
void usbd_add_endpoint(uint8_t b,struct usbd_endpoint *e){(void)b;if(e->ep_addr==USB_I2C_EP_IN)in_ep=e;else out_ep=e;}
void usbd_initialize(uint8_t b,uint32_t reg,void (*handler)(uint8_t,uint8_t)){(void)b;(void)reg;event_handler=handler;}
uint8_t usbd_get_port_speed(uint8_t b){(void)b;return USB_SPEED_HIGH;}
int usbd_ep_start_read(uint8_t b,uint8_t ep,uint8_t *data,uint32_t n)
{(void)b;assert(ep==USB_I2C_EP_OUT && n==512);rx=data;++reads;return 0;}
int usbd_ep_start_write(uint8_t b,uint8_t ep,const uint8_t *data,uint32_t n)
{(void)b;assert(ep==USB_I2C_EP_IN && n==8 && data[0]==1);++writes;return fail_write?-1:0;}
void bridge_ports_abort(void){++aborts;}
void bridge_ports_poll(void){}
void bridge_protocol_start(bridge_protocol_t *p,const uint8_t *req,uint32_t n,uint16_t mps,uint8_t *rsp)
{
    assert(n==8 && mps==512);memset(p,0,sizeof(*p));memcpy(rsp,req,8);p->response=rsp;p->response_len=8;
    ++commits;
    if(reset_in_command)event_handler(0,USBD_EVENT_RESET);
}
bool bridge_protocol_poll(bridge_protocol_t *p){(void)p;return ready;}
static void receive(void)
{assert(rx);memset(rx,0,8);rx[0]=1;out_ep->ep_cb(0,USB_I2C_EP_OUT,8);usb_i2c_task();}
int main(void)
{
    usb_i2c_init(0,0);event_handler(0,USBD_EVENT_CONFIGURED);usb_i2c_task();assert(reads==1);
    receive();assert(writes==1 && reads==1 && commits==1);
    usb_i2c_task();assert(reads==1 && writes==1);
    in_ep->ep_cb(0,USB_I2C_EP_IN,8);usb_i2c_task();assert(reads==2 && writes==1);
    /* An unsolicited stale callback while no IN is active cannot release the
     * next request's reply slot. */
    in_ep->ep_cb(0,USB_I2C_EP_IN,8);receive();usb_i2c_task();assert(writes==2 && reads==2);
    event_handler(0,USBD_EVENT_DISCONNECTED);in_ep->ep_cb(0,USB_I2C_EP_IN,8);usb_i2c_task();assert(reads==2);
    event_handler(0,USBD_EVENT_RESET);event_handler(0,USBD_EVENT_CONFIGURED);usb_i2c_task();assert(reads==3);
    receive();usb_i2c_task();assert(writes==3 && reads==3);
    in_ep->ep_cb(0,USB_I2C_EP_IN,8);usb_i2c_task();assert(reads==4);
    /* Reset after a side effect completed suppresses its old response; it
     * never replays the operation and does not undo the side effect. */
    reset_in_command=true;receive();assert(commits==4 && writes==3);
    usb_i2c_task();assert(commits==4 && writes==3);
    reset_in_command=false;event_handler(0,USBD_EVENT_CONFIGURED);usb_i2c_task();assert(reads==5);
    ready=false;receive();assert(commits==5 && writes==3);
    event_handler(0,USBD_EVENT_RESET);usb_i2c_task();ready=true;usb_i2c_task();assert(writes==3);
    event_handler(0,USBD_EVENT_CONFIGURED);usb_i2c_task();fail_write=true;receive();assert(writes==4 && commits==6);
    usb_i2c_task();usb_i2c_task();assert(writes==4 && commits==6 && aborts>=5);
    puts("PASS actual USB slot: IN lifecycle, epoch reset/disconnect, old completion guards, no replay");
}
