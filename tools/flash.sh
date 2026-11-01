#!/usr/bin/env bash
# Flash an ELF to the HPM6300EVK via OpenOCD (FT2232 probe).
# Usage: tools/flash.sh <firmware.elf>
set -euo pipefail

ELF="${1:?usage: $0 <firmware.elf>}"
[[ -f "$ELF" ]] || { echo "no such file: $ELF" >&2; exit 1; }
ELF="$(cd "$(dirname "$ELF")" && pwd)/$(basename "$ELF")"   # absolute: we cd below
: "${HPM_SDK_BASE:?HPM_SDK_BASE not set; run: source tools/env.sh}"

OPENOCD_DIR="${HPM_SDK_BASE}/boards/openocd"
for f in \
    "${OPENOCD_DIR}/probes/ft2232.cfg" \
    "${OPENOCD_DIR}/soc/hpm6360.cfg" \
    "${OPENOCD_DIR}/boards/hpm6300evk.cfg" \
    "${OPENOCD_DIR}/hpm6300_all_in_one.cfg"; do
    [[ -f "$f" ]] || { echo "missing openocd cfg: $f" >&2; exit 1; }
done

cd "${OPENOCD_DIR}"
exec openocd \
    -c "set HPM_SDK_BASE ${HPM_SDK_BASE}; set BOARD hpm6300evk; set PROBE ft2232;" \
    -f hpm6300_all_in_one.cfg \
    -c "program ${ELF} verify reset exit"
