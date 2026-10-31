# Source this file to set up the HPM toolchain environment:
#   source tools/env.sh
# Each variable can be overridden by exporting it before sourcing.

export HPM_SDK_BASE="${HPM_SDK_BASE:-$HOME/hpm/hpm_sdk}"

# Root of the RISC-V GCC toolchain (the dir that contains bin/riscv32-unknown-elf-gcc)
export GNURISCV_TOOLCHAIN_PATH="${GNURISCV_TOOLCHAIN_PATH:-$HOME/hpm/toolchain/riscv32-unknown-elf-newlib-multilib-2023.10.18-macos-intel}"

# Empty = gnu-gcc (per HPM SDK get_started docs)
export HPM_SDK_TOOLCHAIN_VARIANT="${HPM_SDK_TOOLCHAIN_VARIANT:-}"

# HPM fork of OpenOCD
HPM_OPENOCD_BIN="${HPM_OPENOCD_BIN:-$HOME/hpm/openocd/usr/local/bin}"
case ":$PATH:" in
    *":$HPM_OPENOCD_BIN:"*) ;;
    *) export PATH="$HPM_OPENOCD_BIN:$PATH" ;;
esac

# Note: the toolchain is an Intel (x86_64) build running under Rosetta. Its
# binaries were re-pointed with install_name_tool from /usr/local/opt/... to
# x86_64 Homebrew dylibs in ~/hpm/libdeps/lib (that dir must not be moved or
# deleted). No extra env vars are needed for the compiler itself.

# Raspberry Pi Pico (refgen) toolchain
export PICO_SDK_PATH="${PICO_SDK_PATH:-$HOME/pico/pico-sdk}"
PICO_TC_BIN="${PICO_TC_BIN:-$HOME/pico/toolchain/arm-gnu-toolchain-14.3.rel1-darwin-arm64-arm-none-eabi/bin}"
case ":$PATH:" in
    *":$PICO_TC_BIN:"*) ;;
    *) export PATH="$PICO_TC_BIN:$PATH" ;;
esac
