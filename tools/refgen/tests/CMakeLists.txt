cmake_minimum_required(VERSION 3.16)

project(refgen_tests C)

enable_testing()

set(I2CDEC_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../../../firmware/lib/i2c_decoder)

add_executable(test_pattern
    test_pattern.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../src/pattern.c
    ${I2CDEC_DIR}/src/i2c_decoder.c)
target_include_directories(test_pattern PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
    ${I2CDEC_DIR}/include)
set_target_properties(test_pattern PROPERTIES C_STANDARD 99 C_STANDARD_REQUIRED ON)
target_compile_options(test_pattern PRIVATE -std=c99 -Wall -Wextra -Werror)
if(NOT MSVC)
    target_compile_options(test_pattern PRIVATE -fsanitize=address,undefined -fno-omit-frame-pointer)
    target_link_options(test_pattern PRIVATE -fsanitize=address,undefined)
endif()

add_test(NAME refgen_pattern COMMAND test_pattern)
