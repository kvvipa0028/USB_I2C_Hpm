# i2c-analyzer

Commercial USB I2C bus analyzer. Firmware target: HPM6300EVK (HPM6360, RISC-V).

## Environment

```sh
source tools/env.sh   # HPM_SDK_BASE, GNURISCV_TOOLCHAIN_PATH, OpenOCD in PATH
```

Toolchain lives in `~/hpm/` (outside the repo): `hpm_sdk` v1.13.0,
`toolchain/` (riscv32-unknown-elf-gcc, Intel build under Rosetta with dylibs
re-pointed to `~/hpm/libdeps/lib` via install_name_tool — do not move it),
`openocd/` (HPM fork; needs `brew install libftdi libusb hidapi`).

## Firmware build

```sh
source tools/env.sh
mkdir -p firmware/app/build && cd firmware/app/build
cmake -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip ..
ninja                     # produces output/demo.elf / demo.bin
```

## Flash (requires the board connected via the on-board FT2232)

```sh
tools/flash.sh firmware/app/build/output/demo.elf
```

## usb_speed firmware (USB throughput test)

```sh
source tools/env.sh
mkdir -p firmware/usb_speed/build && cd firmware/usb_speed/build
cmake -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip ..
ninja                     # produces output/demo.elf / demo.bin
```

## refgen (Pico I2C signal generator, tools/refgen/)

Environment: `source tools/env.sh` also exports `PICO_SDK_PATH` (~/pico/pico-sdk,
tag 2.3.1) and prepends the Arm GNU toolchain bin (~/pico/toolchain, gcc 14.3.1).

```sh
mkdir -p tools/refgen/build && cd tools/refgen/build
cmake -GNinja -DPICO_BOARD=pico .. && ninja      # refgen.uf2 (RP2040)
# or: cmake -GNinja -DPICO_BOARD=pico2 .. && ninja   (RP2350)
```

Flash: hold BOOTSEL, plug the Pico, drag `refgen.uf2` onto the RPI-RP2 drive.

Host tests (pattern builder + decoder): `mkdir -p tools/refgen/tests/build &&
cd tools/refgen/tests/build && cmake -GNinja .. && ninja && ctest`.

Pins: CH0 SCL=GP2 SDA=GP3; CH1 SCL=GP4 SDA=GP5; SYNC=GP6 (~1 us pulse at each
pattern loop start, for scope trigger). Push-pull 3.3 V outputs (v1 has no
open-drain mode). **Pico GND must be tied to the EVK GND.**

USB CDC commands (line based, `OK ...` / `ERR <reason>`):
`set <key> <value>` (freq_hz, idle_ns, frames, bytes, nack_every,
restart_every, addr10_every, glitch_line scl|sda|none, glitch_cycles,
glitch_every, offset_ns; -1 disables CH1), `build` (prints period_ns, load%,
segs, min_seg_cycles, res_ns, scl_hz_actual), `start`, `stop`,
`status`, `dump` (expected events CSV `t_ns,type,addr,rw,data,ack,flags`,
ends with a note that expected events assume the analyzer filter removes
glitches).

Waveform engine: each 32-bit segment = bits[31:30] (SDA,SCL) + bits[29:0]
hold; refgen.pio does `out x,30` / `out pins,2` / `jmp x--` — 3-cycle fixed
overhead per segment, min hold 3 cycles, resolution 1 cycle (8 ns @125 MHz,
6.67 ns @150 MHz). Two SMs run the same buffer (DMA data+control channel
loop each); CH1 = CH0 delayed by `offset_ns` via one pre-pushed idle segment
and `pio_enable_sm_mask_in_sync`. CPU idle while running.

## Host tool (Rust)

```sh
cd host
cargo build --release
cargo test
cargo clippy -- -D warnings
./target/release/usb-speed list
./target/release/usb-speed rx --seconds 60 --transfer-size 1048576 --queue 8 [--csv out.csv]
./target/release/usb-speed tx --seconds 60 --transfer-size 1048576 --queue 8
cargo check --target x86_64-pc-windows-msvc   # Windows backend compile check
```

## Decoder library host tests

```sh
mkdir -p firmware/lib/i2c_decoder/build && cd firmware/lib/i2c_decoder/build
cmake -GNinja .. && ninja
ctest --output-on-failure   # 20 cases, ASan+UBSan
./test_i2c_decoder <name>   # run one case (see tests/ for names)
```

Portability check (must stay warning-free):

```sh
riscv32-unknown-elf-gcc -march=rv32imac -mabi=ilp32 -Os -std=c99 \
    -Wall -Wextra -Werror -I firmware/lib/i2c_decoder/include \
    -c firmware/lib/i2c_decoder/src/i2c_decoder.c -o /tmp/i2c_decoder.o
```

## Rules

- Planning docs (`PRD.md`, `DEV_PLAN.md`, `功能清单_UTA0301对标.xlsx`) are
  edited only on explicit request.
- `firmware/lib/i2c_decoder/`: pure C99, no malloc, no global state, no HPM SDK
  deps. Keep per-edge cost low (target ~3M edges/s/channel on HPM6360).
