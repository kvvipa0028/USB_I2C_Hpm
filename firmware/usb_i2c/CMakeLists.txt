# Copyright (c) 2026 赵建智
# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.13)

set(CONFIG_CHERRYUSB 1)
set(CONFIG_CHERRYUSB_DEVICE 1)
set(CONFIG_USB_DEVICE 1)

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(usb_i2c)

# reuse the SDK's shared cherryusb config header
sdk_inc($ENV{HPM_SDK_BASE}/samples/cherryusb/config)
sdk_app_inc(src)
sdk_app_inc(../common)
sdk_app_inc(../boards/evk)
sdk_app_src(src/main.c)
sdk_app_src(src/usb_desc.c)
sdk_app_src(src/usb_i2c.c)
sdk_app_src(src/bridge_protocol.c)
sdk_app_src(src/i2c_port.c)
sdk_app_src(../common/i2c_timing.c)
sdk_app_src(../common/i2c_table.c)
sdk_app_src(../common/i2c_recovery.c)
sdk_app_src(../boards/evk/i2c_board.c)
generate_ide_projects()
