# Copyright (c) 2026
# SPDX-License-Identifier: BSD-3-Clause

cmake_minimum_required(VERSION 3.13)

set(CONFIG_CHERRYUSB 1)
set(CONFIG_CHERRYUSB_DEVICE 1)
set(CONFIG_USB_DEVICE 1)

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(usb_i2c)

# reuse the SDK's shared cherryusb config header
sdk_inc($ENV{HPM_SDK_BASE}/samples/cherryusb/config)
sdk_app_inc(src)
sdk_app_src(src/main.c)
sdk_app_src(src/usb_desc.c)
sdk_app_src(src/usb_i2c.c)
generate_ide_projects()
