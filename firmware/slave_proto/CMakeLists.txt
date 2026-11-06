# Copyright (c) 2026 赵建智
# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.13)

set(SLV_MODE "i2c_mode_normal" CACHE STRING "I2C slave mode for S1 bench tests")
set_property(CACHE SLV_MODE PROPERTY STRINGS i2c_mode_normal i2c_mode_fast i2c_mode_fast_plus)
if(NOT SLV_MODE MATCHES "^(i2c_mode_normal|i2c_mode_fast|i2c_mode_fast_plus)$")
    message(FATAL_ERROR "SLV_MODE must be i2c_mode_normal, i2c_mode_fast, or i2c_mode_fast_plus")
endif()

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(slave_proto)

sdk_app_src(src/main.c)
target_compile_definitions(app PRIVATE SLV_MODE=${SLV_MODE})
generate_ide_projects()
