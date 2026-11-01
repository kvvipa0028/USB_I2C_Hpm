# i2c-analyzer

Commercial USB-to-I2C interface device (hardware I2C, each port master or
slave). Firmware target: HPM6300EVK (HPM6360, RISC-V).

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

Reading firmware results without a console: JTAG,
`openocd ... -c init -c halt -c "mdw <addr> <n>" -c resume -c shutdown`
(address from `riscv32-unknown-elf-nm demo.elf`). Each command needs its own
`-c`; commands packed into one `-c` after `halt` do not run.

## usb_speed firmware (USB throughput test)

```sh
source tools/env.sh
mkdir -p firmware/usb_speed/build && cd firmware/usb_speed/build
cmake -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip ..
ninja                     # produces output/demo.elf / demo.bin
```

## i2c_proto firmware (T1.8 I2C master bring-up)

Peer: HPM6750EVKMini running the SDK sample `samples/drivers/i2c/interrupt/slave`
(address 0x16, echoes 128 bytes). Wiring: EVK J28[13]/[15] to EVKMini
P1[13]/[15], GND to GND.

```sh
source tools/env.sh
cmake -S firmware/i2c_proto -B firmware/i2c_proto/build -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
ninja -C firmware/i2c_proto/build
cmake -S $HPM_SDK_BASE/samples/drivers/i2c/interrupt/slave -B /tmp/i2c_slave_evkmini -GNinja -DBOARD=hpm6750evkmini -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
ninja -C /tmp/i2c_slave_evkmini
```

Both FT2232 probes report the same serial and both chips the same JTAG ID, so
select the board by USB location. With the current ports, EVK = `32-2.4` and
EVKMini = `32-2.1.4`, derived from `/dev/cu.usbserial-202400` and
`/dev/cu.usbserial-2021400`. Flash the slave first:

```sh
cd $HPM_SDK_BASE/boards/openocd
openocd -c "set HPM_SDK_BASE $HPM_SDK_BASE; set BOARD hpm6750evkmini; set PROBE ft2232;" -f hpm6750_all_in_one.cfg -c "adapter usb location 32-2.1.4" -c "program /tmp/i2c_slave_evkmini/output/demo.elf verify reset exit"
openocd -c "set HPM_SDK_BASE $HPM_SDK_BASE; set BOARD hpm6300evk; set PROBE ft2232;" -f hpm6300_all_in_one.cfg -c "adapter usb location 32-2.4" -c "program <repo>/firmware/i2c_proto/build/output/demo.elf verify reset" -c "sleep 2000" -c halt -c "mdw <g_result> 5" -c resume -c shutdown
```

Pass: `c0ffee18 00000000 00000000 00000000 000007d3` (done, write ok, read ok,
0 mismatches, absent address returns `status_i2c_no_addr_hit` = 2003).

## slave_proto / slave_test firmware (T1.9 register-table slave)

EVK runs `firmware/slave_proto` (I2C0 slave at 0x50, 256-byte EEPROM-style
table, `g_table`); EVKMini runs `tools/slave_test` (external master, 200 rounds
at 100 kHz, compares every read with its model `g_exp`). Same wiring as i2c_proto.

```sh
cmake -S firmware/slave_proto -B firmware/slave_proto/build -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release && ninja -C firmware/slave_proto/build
cmake -S tools/slave_test -B tools/slave_test/build -GNinja -DBOARD=hpm6750evkmini -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release && ninja -C tools/slave_test/build
```

Bus speed: `SLV_MODE` (slave_proto) and `BUS_MODE` (slave_test) must match
(`i2c_mode_normal` = 100k, `i2c_mode_fast` = 400k). `PEER_PULLUP=1` also turns
on the EVKMini's internal pull-ups.

Flash the EVK (slave) first, then the EVKMini, with the openocd lines from the
i2c_proto section; read `g_result` (9 words) on the EVKMini after ~25 s. Pass:
magic `c0ffee19`, rounds `c8`, then four zeros (call_fail, mismatches,
first_info, first_val), and `g_table` on the EVK equals `g_exp`. The last three
words are mode, elapsed_us, peer_pullup. Test record: `docs/T1.9-slave-test.md`,
raw openocd output in `docs/T1.9-data/`. `SEQ_ONLY=1` in slave_test instead
writes 0..254 at offset 0 once (quick receive check). Symbol addresses change
between builds; take them from `riscv32-unknown-elf-nm`.

Enabling the BYTERECV interrupt turns hardware auto-ACK off (INTEN register
doc), so the slave must ACK each received byte with `i2c_respond_ack()`;
without it the slave holds SCL low after the first data byte. The SDK helper
`i2c_enable_auto_ack()` just sets BYTERECV, so it turns hardware auto-ACK off.

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

## Rules

- Planning docs (`PRD.md`, `DEV_PLAN.md`, `功能清单_UTA0301对标.xlsx`) are
  edited only on explicit request.
- HPM SDK builds `-O0` unless `-DCMAKE_BUILD_TYPE=release` is passed to cmake
  (measured ~3.5x slower on CPU-bound code). Pass it whenever performance matters.
