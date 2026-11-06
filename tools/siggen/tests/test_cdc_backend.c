#include "usbd_core.h"
#include "cdc_acm.h"
#include "siggen_board.h"
#include <assert.h>
#include <stdio.h>
static void (*event)(uint8_t, uint8_t);
static struct usbd_endpoint *out_ep, *in_ep;
static uint8_t *rx;
static uint32_t tx_size, tx_calls;
static uint16_t mps = 64;
static bool fail_write;
static unsigned locked;
extern void usbd_cdc_acm_set_dtr(uint8_t, uint8_t, bool);
uint32_t siggen_board_lock(void) { return locked++; }
void siggen_board_unlock(uint32_t prior) { assert(locked > 0); locked = prior; }
void usbd_desc_register(uint8_t b, const struct usb_descriptor *d) { (void)b; assert(d != NULL); }
void usbd_add_interface(uint8_t b, struct usbd_interface *i) { (void)b; (void)i; }
struct usbd_interface *usbd_cdc_acm_init_intf(uint8_t b, struct usbd_interface *i) { (void)b; return i; }
void usbd_add_endpoint(uint8_t b, struct usbd_endpoint *ep)
{
    (void)b; if ((ep->ep_addr & 0x80U) != 0) in_ep = ep; else out_ep = ep;
}
void usbd_initialize(uint8_t b, uint32_t base, void (*cb)(uint8_t, uint8_t)) { (void)b; (void)base; event = cb; }
uint16_t usbd_get_ep_mps(uint8_t b, uint8_t ep) { (void)b; (void)ep; return mps; }
int usbd_ep_start_read(uint8_t b, uint8_t ep, uint8_t *data, uint32_t size)
{ (void)b; (void)ep; assert(size == mps); rx = data; return 0; }
int usbd_ep_start_write(uint8_t b, uint8_t ep, const uint8_t *data, uint32_t size)
{ (void)b; (void)ep; assert(data != NULL || size == 0); ++tx_calls; tx_size = size; return fail_write ? -1 : 0; }
static void connected(void)
{
    event(0, USBD_EVENT_CONFIGURED);
    usbd_cdc_acm_set_dtr(0, 0, true);
    (void)cdc_take_session_lost();
}
static void receive(const char *data, size_t size)
{
    assert(size <= mps); memcpy(rx, data, size); out_ep->ep_cb(0, out_ep->ep_addr, (uint32_t)size);
}
static void complete(uint32_t size) { in_ep->ep_cb(0, in_ep->ep_addr, size); }
int main(void)
{
    cdc_acm_init(0, 0); connected(); assert(cdc_can_write());
    assert(cdc_write("OK\n", 3)); assert(!cdc_can_write());
    assert(!cdc_write("second\n", 7) && tx_calls == 1);
    /* DTR does not cancel DMA: reopening cannot overwrite the old IN buffer. */
    usbd_cdc_acm_set_dtr(0, 0, false); assert(cdc_take_session_lost());
    usbd_cdc_acm_set_dtr(0, 0, true); assert(!cdc_can_write());
    complete(3); assert(cdc_can_write());
    /* A response prepared before DTR loss cannot enter the new session. */
    usbd_cdc_acm_set_dtr(0, 0, false); usbd_cdc_acm_set_dtr(0, 0, true);
    uint32_t before = tx_calls;
    assert(!cdc_can_write() && !cdc_write("old\n", 4) && tx_calls == before);
    assert(cdc_take_session_lost() && cdc_can_write());
    assert(cdc_write("new\n", 4)); complete(4);
    char packet[512]; memset(packet, 'x', sizeof(packet));
    assert(cdc_write(packet, 64)); complete(64); assert(tx_size == 0 && !cdc_can_write());
    complete(0); assert(cdc_can_write());
    /* Reset clears queued input; stale completion while unconfigured is inert. */
    receive("start\n", 6); assert(cdc_write("old\n", 4));
    event(0, USBD_EVENT_RESET); assert(cdc_take_session_lost() && cdc_getchar() == -1);
    before = tx_calls; complete(4); assert(tx_calls == before && !cdc_can_write());
    connected(); assert(cdc_can_write());
    event(0, USBD_EVENT_RESET);
    event(0, USBD_EVENT_CONFIGURED); usbd_cdc_acm_set_dtr(0, 0, true);
    before = tx_calls;
    assert(!cdc_can_write() && !cdc_write("old\n", 4) && tx_calls == before);
    assert(cdc_take_session_lost() && cdc_can_write());
    assert(cdc_write("new\n", 4)); complete(4);
    /* RX overflow remains visible while the previous response is unread. */
    assert(cdc_write("busy\n", 5));
    for (unsigned i = 0; i < 17; ++i) receive(packet, 64);
    assert(cdc_take_session_lost()); assert(!cdc_can_write()); assert(cdc_getchar() == -2);
    receive("start\nstatus\n", 13);
    const char *expected = "status\n";
    for (size_t i = 0; expected[i] != '\0'; ++i) assert(cdc_getchar() == expected[i]);
    assert(cdc_getchar() == -1); complete(5); assert(cdc_can_write());
    /* Overflow also gates a previously prepared reply without a busy IN. */
    for (unsigned i = 0; i < 17; ++i) receive(packet, 64);
    before = tx_calls;
    assert(!cdc_can_write() && !cdc_write("old\n", 4) && tx_calls == before);
    assert(cdc_take_session_lost() && cdc_can_write());
    assert(cdc_getchar() == -2);
    fail_write = true; assert(!cdc_write("fault\n", 6)); assert(cdc_take_session_lost() && !cdc_can_write());
    fail_write = false; event(0, USBD_EVENT_RESET); connected();
    /* A closed terminal cannot queue commands for a future DTR session. */
    usbd_cdc_acm_set_dtr(0, 0, false); receive("start\n", 6);
    usbd_cdc_acm_set_dtr(0, 0, true); assert(cdc_getchar() == -1 && cdc_take_session_lost());
    mps = 512; event(0, USBD_EVENT_RESET); connected();
    assert(cdc_write(packet, 512)); complete(512); assert(tx_size == 0 && !cdc_can_write());
    complete(0); assert(cdc_can_write() && locked == 0);
    puts("siggen production CDC backend tests passed");
    return 0;
}
