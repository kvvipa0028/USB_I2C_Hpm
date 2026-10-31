cmake_minimum_required(VERSION 3.13)

include($ENV{PICO_SDK_PATH}/external/pico_sdk_import.cmake)

project(refgen C CXX ASM)
pico_sdk_init()

add_executable(refgen
    src/main.c
    src/pattern.c)

pico_generate_pio_header(refgen ${CMAKE_CURRENT_LIST_DIR}/src/refgen.pio)

target_include_directories(refgen PRIVATE
    src
    ${CMAKE_CURRENT_LIST_DIR}/../../firmware/lib/i2c_decoder/include)

target_link_libraries(refgen
    pico_stdlib
    hardware_pio
    hardware_dma
    hardware_gpio)

pico_enable_stdio_usb(refgen 1)
pico_enable_stdio_uart(refgen 0)

target_compile_options(refgen PRIVATE -Wall -Wextra -Werror)

pico_add_extra_outputs(refgen)
