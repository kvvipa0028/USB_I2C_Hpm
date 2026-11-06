# Copyright (c) 2026 赵建智
# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.13)

set(BUS_MODE "i2c_mode_normal" CACHE STRING "I2C master mode for S1 bench tests")
set_property(CACHE BUS_MODE PROPERTY STRINGS i2c_mode_normal i2c_mode_fast i2c_mode_fast_plus)
if(NOT BUS_MODE MATCHES "^(i2c_mode_normal|i2c_mode_fast|i2c_mode_fast_plus)$")
    message(FATAL_ERROR "BUS_MODE must be i2c_mode_normal, i2c_mode_fast, or i2c_mode_fast_plus")
endif()

set(ROUNDS "200" CACHE STRING "Test rounds (1..65535; preserves the result layout)")
if(NOT ROUNDS MATCHES "^[1-9][0-9]*$" OR ROUNDS GREATER 65535)
    message(FATAL_ERROR "ROUNDS must be a decimal integer from 1 to 65535 without leading zeros")
endif()

set(PEER_PULLUP "0" CACHE STRING "Enable EVKMini internal pull-ups (0 or 1)")
set_property(CACHE PEER_PULLUP PROPERTY STRINGS 0 1)
if(NOT PEER_PULLUP MATCHES "^[01]$")
    message(FATAL_ERROR "PEER_PULLUP must be 0 or 1")
endif()

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(slave_test)

sdk_app_src(src/main.c)
target_compile_definitions(app PRIVATE BUS_MODE=${BUS_MODE} ROUNDS=${ROUNDS}U PEER_PULLUP=${PEER_PULLUP})
generate_ide_projects()
