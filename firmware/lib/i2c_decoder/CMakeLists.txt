cmake_minimum_required(VERSION 3.16)

project(i2c_decoder C)

add_library(i2c_decoder STATIC src/i2c_decoder.c)
target_include_directories(i2c_decoder PUBLIC include)
set_target_properties(i2c_decoder PROPERTIES C_STANDARD 99 C_STANDARD_REQUIRED ON)
target_compile_options(i2c_decoder PRIVATE -Wall -Wextra -Werror)

if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    set(I2CDEC_TESTS_DEFAULT ON)
else()
    set(I2CDEC_TESTS_DEFAULT OFF)
endif()
option(I2CDEC_BUILD_TESTS "Build i2c_decoder host unit tests" ${I2CDEC_TESTS_DEFAULT})

if(I2CDEC_BUILD_TESTS)
    enable_testing()

    add_executable(test_i2c_decoder tests/test_i2c_decoder.c tests/wavegen.c)
    target_include_directories(test_i2c_decoder PRIVATE tests)
    target_link_libraries(test_i2c_decoder PRIVATE i2c_decoder)
    set_target_properties(test_i2c_decoder PROPERTIES C_STANDARD 99 C_STANDARD_REQUIRED ON)

    target_compile_options(i2c_decoder PRIVATE -std=c99)
    target_compile_options(test_i2c_decoder PRIVATE -std=c99 -Wall -Wextra -Werror)
    if(NOT MSVC)
        set(I2CDEC_SANITIZE_FLAGS -fsanitize=address,undefined -fno-omit-frame-pointer)
        target_compile_options(i2c_decoder PRIVATE ${I2CDEC_SANITIZE_FLAGS})
        target_compile_options(test_i2c_decoder PRIVATE ${I2CDEC_SANITIZE_FLAGS})
        target_link_options(test_i2c_decoder PRIVATE -fsanitize=address,undefined)
    endif()

    set(I2CDEC_CASES
        write read restart addr_nack addr10_write addr10_read
        addr10_partial_stop addr10_partial_restart tie_order sda_glitch
        glitch_filtered glitch_boundary byte_interrupted unsynced
        stuck_sda level_repeat stretch timings chunking fuzz)
    foreach(c ${I2CDEC_CASES})
        add_test(NAME i2cdec_${c} COMMAND test_i2c_decoder ${c})
    endforeach()
endif()
