#include "siggen_recovery_gpio.h"
#include "siggen_board.h"
#include "board.h"
#include "hpm_gpio_drv.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

#define SCL (UINT32_C(1) << 11)
#define SDA (UINT32_C(1) << 10)
#define OTHER (UINT32_C(1) << 5)
GPIO_Type test_gpio;
static siggen_peer_t peer;
static uint64_t now;
static uint32_t sequence, configured_at, cleared_at, releases, configs;
static bool config_ok;
extern void siggen_scl_rising_isr(void);

static void pin(GPIO_Type *g, uint32_t port, uint8_t index)
{ assert(g == HPM_GPIO0 && port == GPIO_DI_GPIOB && index == 11); }
void gpio_disable_pin_interrupt(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); g->enabled &= ~SCL; }
void gpio_clear_pin_interrupt_flag(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); g->flags &= ~SCL; cleared_at = ++sequence; }
void gpio_enable_pin_interrupt(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); assert(cleared_at > configured_at && (g->flags & SCL) == 0); g->enabled |= SCL; }
void gpio_config_pin_interrupt(GPIO_Type *g, uint32_t port, uint8_t index, gpio_interrupt_trigger_t trigger)
{
    pin(g, port, index); assert(trigger == gpio_interrupt_trigger_edge_rising);
    configured_at = ++sequence;
    g->flags |= SCL; /* Configuration/stale pending must be cleared before arm. */
}
bool gpio_check_pin_interrupt_flag(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); return (g->flags & SCL) != 0; }
bool gpio_check_pin_interrupt_enabled(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); return (g->enabled & SCL) != 0; }
uint8_t gpio_read_pin(GPIO_Type *g, uint32_t port, uint8_t index)
{ pin(g, port, index); return (uint8_t)((g->input & SCL) != 0); }
void gpio_set_pin_input(GPIO_Type *g, uint32_t port, uint8_t index)
{ assert(g == HPM_GPIO0 && port == GPIO_DO_GPIOB && index == 10); g->output &= ~SDA; ++releases; }
void intc_m_enable_irq_with_priority(uint32_t irq, uint32_t priority)
{ assert(irq == IRQn_GPIO0_B && priority == 1); }
uint64_t siggen_board_now_us(void) { return now; }

static void disable(void *context) { (void)context; siggen_recovery_gpio_disarm(); }
static void unhold(void *context)
{ (void)context; siggen_recovery_gpio_disarm(); test_gpio.output &= ~(SCL | SDA); }
static bool configure(void *context, bool slave, uint8_t address, uint32_t hz, bool nack)
{ (void)context; (void)slave; (void)address; (void)hz; (void)nack; ++configs; return config_ok; }
static bool busy(void *context) { (void)context; return false; }
static void hold(void *context, uint8_t mask) { (void)context; (void)mask; }
static uint8_t lines(void *context) { (void)context; return 3; }
static void ack(void *context, bool yes) { (void)context; (void)yes; }
static bool valid(void *context) { (void)context; return true; }
static bool master_begin(void *context, bool read, uint8_t address)
{ (void)context; (void)read; (void)address; return false; }
static bool master_poll(void *context, bool read, siggen_result_t *result, uint8_t bytes[2])
{ (void)context; (void)read; (void)result; (void)bytes; return false; }
static bool hold_clocks(void *context, uint32_t generation)
{
    disable(context);
    test_gpio.output = (test_gpio.output & ~SCL) | SDA;
    return siggen_recovery_gpio_arm(&peer, generation);
}
static void command(const char *text, const char *expected)
{
    siggen_command_t c; siggen_reply_t reply; char out[64];
    assert(siggen_command_parse(text, &c));
    siggen_peer_prepare(&peer, &c, now, &reply);
    siggen_peer_format(&reply, out, sizeof(out));
    assert(strcmp(out, expected) == 0);
}
static void setup(void)
{
    memset(&test_gpio, 0, sizeof(test_gpio));
    sequence = configured_at = cleared_at = releases = configs = 0;
    now = 0; config_ok = true;
    siggen_hal_t hal = { NULL, configure, disable, busy, hold, unhold, lines, ack,
        valid, master_begin, master_poll, hold_clocks };
    siggen_peer_init(&peer, &hal);
    test_gpio.input = SCL;
    test_gpio.enabled = OTHER;
    test_gpio.flags = OTHER | SCL;
    command("slave 0x50 100000", "OK\n");
}
static void rising(void)
{
    now += 10;
    test_gpio.input |= SCL; test_gpio.flags |= SCL;
    siggen_scl_rising_isr();
}
int main(void)
{
    setup(); command("hold_sda_clocks 9 1000", "OK\n");
    assert(peer.clock_observed == 0 && (test_gpio.output & SDA) != 0 && (test_gpio.output & SCL) == 0);
    assert((test_gpio.flags & SCL) == 0 && (test_gpio.flags & OTHER) != 0);
    siggen_scl_rising_isr(); /* Initial high/shared pin flag cannot count. */
    assert(peer.clock_observed == 0 && releases == 0);
    for (unsigned i = 0; i < 8; ++i) rising();
    assert(peer.clock_observed == 8 && releases == 0 && (test_gpio.output & SDA) != 0);
    uint32_t before = configs;
    rising();
    assert(releases == 1 && (test_gpio.output & SDA) == 0 && (test_gpio.enabled & SCL) == 0);
    assert((test_gpio.flags & OTHER) != 0 && (test_gpio.enabled & OTHER) != 0);
    assert(peer.clock_result == SIG_CLOCK_COMPLETED && peer.clock_restore_pending && configs == before);
    rising(); assert(releases == 1 && peer.clock_observed == 9);
    siggen_peer_poll(&peer, ++now); assert(configs == before + 1U && !peer.clock_restore_pending);
    siggen_peer_poll(&peer, ++now); assert(configs == before + 1U);

    command("hold_sda_clocks 9 1000", "OK\n");
    uint32_t old_generation = peer.clock_generation;
    for (unsigned i = 0; i < 8; ++i) rising();
    siggen_peer_stop(&peer);
    assert((test_gpio.output & SDA) == 0 && (test_gpio.enabled & SCL) == 0);
    test_gpio.flags |= SCL; siggen_scl_rising_isr(); assert(peer.clock_observed == 8);
    command("hold_sda_clocks 9 1000", "OK\n");
    assert((test_gpio.flags & SCL) == 0 && peer.clock_observed == 0);
    assert(!siggen_peer_scl_rising(&peer, old_generation, now));
    siggen_scl_rising_isr(); assert(peer.clock_observed == 0);
    siggen_peer_poll(&peer, now + 1000);
    assert(peer.clock_result == SIG_CLOCK_TIMEOUT && (test_gpio.output & SDA) == 0 && (test_gpio.enabled & SCL) == 0);

    setup(); test_gpio.input &= ~SCL;
    command("hold_sda_clocks 9 1000", "ERR clock arm; released\n");
    assert(peer.clock_result == SIG_CLOCK_ERROR && (test_gpio.output & SDA) == 0 && (test_gpio.enabled & SCL) == 0);
    assert((test_gpio.flags & OTHER) != 0 && (test_gpio.enabled & OTHER) != 0);
    puts("siggen production recovery GPIO IRQ tests passed");
    return 0;
}
