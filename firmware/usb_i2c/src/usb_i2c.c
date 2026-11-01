/*
 * usb_i2c bridge protocol (prototype; the reviewed protocol is DEV_PLAN T2.2).
 * One request per bulk OUT transfer, one reply per bulk IN transfer.
 *
 * Request:  cmd u8, seq u8, addr u8, rsvd u8, wlen u16 LE, rlen u16 LE, wdata[wlen]
 * Reply:    cmd u8, seq u8 (echoed), status u8, rsvd u8, hpm_stat u32 LE, data
 *
 * cmd 0x01 PING: data = protocol version u8 + firmware id (ASCII).
 * cmd 0x10 I2C:  on I2C0 at 7-bit addr: wlen>0,rlen=0 write; wlen=0,rlen>0 read;
 *                both > 0: write then repeated START then read. 1..256 bytes each.
 *                data = rlen bytes read (only when status is OK).
 * The reply is never longer than the host asked for (8 + rlen, or 8 + 64 for PING),
 * and trailing request bytes beyond 8 + wlen are ignored (host padding).
 */

#include <string.h>

#include "board.h"
#include "usbd_core.h"
#include "hpm_i2c_drv.h"
#include "usb_i2c.h"

#define PROTO_VERSION 1U
#define FW_ID         "usb_i2c 0.1"
#define CMD_PING      0x01U
#define CMD_I2C       0x10U
#define ST_OK         0U
#define ST_NACK       1U
#define ST_TIMEOUT    2U
#define ST_BAD_REQ    3U
#define ST_I2C_ERR    4U
#define HDR           8U
#define MAX_XFER      256U
#define BUF_BYTES     512U   /* one HS packet; requests and replies are <= 264 bytes */

USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t rx_buf[BUF_BYTES];
USB_NOCACHE_RAM_SECTION USB_MEM_ALIGNX static uint8_t tx_buf[BUF_BYTES];

static uint8_t s_busid;
static volatile uint8_t configured;
static volatile uint8_t rx_ready;
static volatile uint32_t rx_len;

static void usbd_event_handler(uint8_t busid, uint8_t event)
{
    switch (event) {
    case USBD_EVENT_RESET:
    case USBD_EVENT_DISCONNECTED:
        configured = 0;
        rx_ready = 0;
        break;
    case USBD_EVENT_CONFIGURED:
        configured = 1;
        rx_ready = 0;
        usbd_ep_start_read(busid, USB_I2C_EP_OUT, rx_buf, BUF_BYTES);
        break;
    default:
        break;
    }
}

static void ep_out_cb(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid;
    (void)ep;
    rx_len = nbytes;
    rx_ready = 1;   /* handled in usb_i2c_task(); OUT is re-armed after the reply */
}

static void ep_in_cb(uint8_t busid, uint8_t ep, uint32_t nbytes)
{
    (void)busid;
    (void)ep;
    (void)nbytes;
}

static struct usbd_endpoint out_ep = { .ep_addr = USB_I2C_EP_OUT, .ep_cb = ep_out_cb };
static struct usbd_endpoint in_ep = { .ep_addr = USB_I2C_EP_IN, .ep_cb = ep_in_cb };
static struct usbd_interface intf;

static uint8_t map_status(hpm_stat_t st)
{
    if (st == status_success) {
        return ST_OK;
    }
    if (st == status_i2c_no_addr_hit || st == status_i2c_no_ack) {
        return ST_NACK;
    }
    if (st == status_timeout) {
        return ST_TIMEOUT;
    }
    return ST_I2C_ERR;
}

/* returns the reply length */
static uint32_t handle(uint8_t *req, uint32_t len, uint8_t *rsp)
{
    memset(rsp, 0, HDR);
    if (len < HDR) {
        rsp[2] = ST_BAD_REQ;
        return HDR;
    }
    rsp[0] = req[0];
    rsp[1] = req[1];
    rsp[2] = ST_BAD_REQ;

    if (req[0] == CMD_PING) {
        rsp[2] = ST_OK;
        rsp[HDR] = PROTO_VERSION;
        memcpy(&rsp[HDR + 1], FW_ID, sizeof(FW_ID) - 1);
        return HDR + 1 + sizeof(FW_ID) - 1;
    }
    if (req[0] == CMD_I2C) {
        uint8_t addr = req[2];
        uint32_t wlen = req[4] | ((uint32_t)req[5] << 8);
        uint32_t rlen = req[6] | ((uint32_t)req[7] << 8);
        uint8_t *wdata = &req[HDR];
        hpm_stat_t st;

        if (addr > 0x7FU || wlen > MAX_XFER || rlen > MAX_XFER ||
            (wlen == 0U && rlen == 0U) || len < HDR + wlen) {
            return HDR;
        }
        if (rlen == 0U) {
            st = i2c_master_write(BOARD_APP_I2C_BASE, addr, wdata, wlen);
        } else if (wlen == 0U) {
            st = i2c_master_read(BOARD_APP_I2C_BASE, addr, &rsp[HDR], rlen);
        } else {
            st = i2c_master_address_read(BOARD_APP_I2C_BASE, addr, wdata, wlen, &rsp[HDR], rlen);
        }
        rsp[2] = map_status(st);
        rsp[4] = (uint8_t)st;
        rsp[5] = (uint8_t)(st >> 8);
        rsp[6] = (uint8_t)(st >> 16);
        rsp[7] = (uint8_t)(st >> 24);
        return (rsp[2] == ST_OK) ? HDR + rlen : HDR;
    }
    return HDR;
}

void usb_i2c_task(void)
{
    uint32_t n;

    if (!rx_ready) {
        return;
    }
    n = handle(rx_buf, rx_len, tx_buf);
    rx_ready = 0;
    if (configured) {
        usbd_ep_start_write(s_busid, USB_I2C_EP_IN, tx_buf, n);
        usbd_ep_start_read(s_busid, USB_I2C_EP_OUT, rx_buf, BUF_BYTES);
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
