# Copyright (c) 2026 赵建智
# SPDX-License-Identifier: MIT

cmake_minimum_required(VERSION 3.13)

find_package(hpm-sdk REQUIRED HINTS $ENV{HPM_SDK_BASE})

project(slave_test)

sdk_app_src(src/main.c)
generate_ide_projects()
