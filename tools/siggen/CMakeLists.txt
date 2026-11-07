# Copyright (c) 2026 赵建智
# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.13)

set(CONFIG_CHERRYUSB 1)
set(CONFIG_USB_DEVICE 1)
set(CONFIG_USB_DEVICE_CDC_ACM 1)

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(sig5301)

if(NOT BOARD STREQUAL "hpm6750evkmini")
    message(FATAL_ERROR "siggen fault injection is mapped only for hpm6750evkmini PB11/PB10")
endif()

sdk_inc($ENV{HPM_SDK_BASE}/samples/cherryusb/config)
sdk_app_inc(src)
sdk_app_src(src/main.c)
sdk_app_src(src/cdc_acm.c)
sdk_app_src(src/cdc_buffer.c)
sdk_app_src(src/siggen_command.c)
sdk_app_src(src/siggen_peer.c)
sdk_app_src(src/siggen_session.c)
sdk_app_src(src/siggen_board.c)
sdk_app_src(src/siggen_recovery_gpio.c)
generate_ide_projects()
