#ifndef SIGGEN_TEST_GPIO_H
#define SIGGEN_TEST_GPIO_H
#include <stdbool.h>
#include <stdint.h>
typedef struct { uint32_t flags, enabled, input, output; } GPIO_Type;
extern GPIO_Type test_gpio;
#define HPM_GPIO0 (&test_gpio)
#define GPIO_DI_GPIOB 1U
#define GPIO_DO_GPIOB 1U
typedef enum { gpio_interrupt_trigger_edge_rising } gpio_interrupt_trigger_t;
void gpio_disable_pin_interrupt(GPIO_Type *, uint32_t, uint8_t);
void gpio_clear_pin_interrupt_flag(GPIO_Type *, uint32_t, uint8_t);
void gpio_enable_pin_interrupt(GPIO_Type *, uint32_t, uint8_t);
void gpio_config_pin_interrupt(GPIO_Type *, uint32_t, uint8_t, gpio_interrupt_trigger_t);
bool gpio_check_pin_interrupt_flag(GPIO_Type *, uint32_t, uint8_t);
bool gpio_check_pin_interrupt_enabled(GPIO_Type *, uint32_t, uint8_t);
uint8_t gpio_read_pin(GPIO_Type *, uint32_t, uint8_t);
void gpio_set_pin_input(GPIO_Type *, uint32_t, uint8_t);
#endif
