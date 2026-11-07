#ifndef BRIDGE_EVK_I2C_BOARD_H
#define BRIDGE_EVK_I2C_BOARD_H
#include "board.h"
#include "hpm_i2c_drv.h"
#include "hpm_gpio_drv.h"
typedef struct {
    I2C_Type *base;
    uint32_t irq, scl_pad, sda_pad, scl_func, sda_func, scl_pin, sda_pin;
} bridge_board_port_t;
extern const bridge_board_port_t bridge_board_ports[2];
void bridge_board_mux(uint8_t port, bool gpio);
void bridge_board_drive(uint8_t port, bool scl_low, bool sda_low);
uint8_t bridge_board_lines(uint8_t port);
#endif
