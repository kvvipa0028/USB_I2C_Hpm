#!/usr/bin/env bash
# Build the S1 slave/peer pair at 100k, 400k and 1M. Never connects to a board.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: tools/s1-build-matrix.sh [--build-root DIR] [--rounds N] [--peer-pullup 0|1] [--jobs N]

Builds six release targets in a fresh run directory under build/s1-matrix.
ROUNDS defaults to 200 (range 1..65535); PEER_PULLUP defaults to 0.
Saves build logs, compile commands, symbols, source/ELF hashes and a summary.
Does not flash, run OpenOCD, or claim hardware timing/data-integrity results.
EOF
}

die() {
    echo "$*" >&2
    exit 2
}

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$repo_dir/build/s1-matrix"
rounds=200
peer_pullup=0
jobs=4

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --build-root|--rounds|--peer-pullup|--jobs)
            [[ $# -ge 2 ]] || die "missing value for $1"
            case "$1" in
                --build-root) build_root="$2" ;;
                --rounds) rounds="$2" ;;
                --peer-pullup) peer_pullup="$2" ;;
                --jobs) jobs="$2" ;;
            esac
            shift 2
            ;;
        *) die "unknown option: $1 (use --help)" ;;
    esac
done

[[ -n "$build_root" ]] || die "build root must not be empty"
[[ "$rounds" =~ ^[1-9][0-9]{0,4}$ ]] || die "rounds must be 1..65535 without leading zeros"
(( rounds <= 65535 )) || die "rounds must be 1..65535"
[[ "$peer_pullup" =~ ^[01]$ ]] || die "peer-pullup must be 0 or 1"
[[ "$jobs" =~ ^[1-9][0-9]{0,2}$ ]] || die "jobs must be 1..128 without leading zeros"
(( jobs <= 128 )) || die "jobs must be 1..128"

source "$repo_dir/tools/env.sh"
for tool in cmake ninja git; do
    command -v "$tool" >/dev/null || die "missing tool: $tool"
done
gcc="$GNURISCV_TOOLCHAIN_PATH/bin/riscv32-unknown-elf-gcc"
nm="$GNURISCV_TOOLCHAIN_PATH/bin/riscv32-unknown-elf-nm"
[[ -x "$gcc" && -x "$nm" ]] || die "RISC-V gcc/nm not found under GNURISCV_TOOLCHAIN_PATH"
[[ -f "$HPM_SDK_BASE/VERSION" ]] || die "HPM SDK VERSION file not found"
if command -v shasum >/dev/null; then
    hash_cmd=(shasum -a 256)
elif command -v sha256sum >/dev/null; then
    hash_cmd=(sha256sum)
else
    die "missing SHA-256 tool (shasum or sha256sum)"
fi

mkdir -p "$build_root"
build_root="$(cd "$build_root" && pwd)"
run_dir="$(mktemp -d "$build_root/run-XXXXXXXX")"
echo "Build evidence: $run_dir"
{
    printf 'repository=%s\n' "$repo_dir"
    printf 'repository_head=%s\n' "$(git -C "$repo_dir" rev-parse HEAD)"
    printf 'sdk=%s\n' "$HPM_SDK_BASE"
    if git -C "$HPM_SDK_BASE" rev-parse HEAD >/dev/null 2>&1; then
        printf 'sdk_head=%s\n' "$(git -C "$HPM_SDK_BASE" rev-parse HEAD)"
        git -C "$HPM_SDK_BASE" status --short -- drivers/src/hpm_i2c_drv.c drivers/inc/hpm_i2c_drv.h boards/hpm6300evk boards/hpm6750evkmini
    fi
    cat "$HPM_SDK_BASE/VERSION"
    "$gcc" --version
    printf 'evidence_kind=target_compile_only\nbuild_type=release\nrounds=%s\npeer_pullup=%s\nhardware_status=PENDING_HARDWARE\n' "$rounds" "$peer_pullup"
} > "$run_dir/environment.txt"
"${hash_cmd[@]}" \
    "$repo_dir/firmware/slave_proto/CMakeLists.txt" \
    "$repo_dir/firmware/slave_proto/src/main.c" \
    "$repo_dir/tools/slave_test/CMakeLists.txt" \
    "$repo_dir/tools/slave_test/src/main.c" \
    "$repo_dir/tools/s1-build-matrix.sh" > "$run_dir/source-sha256.txt"
printf 'target\tboard\tmode\trounds\tpeer_pullup\tbuild_status\thardware_status\n' > "$run_dir/summary.tsv"

for rate in 100k 400k 1m; do
    case "$rate" in
        100k) mode=i2c_mode_normal ;;
        400k) mode=i2c_mode_fast ;;
        1m) mode=i2c_mode_fast_plus ;;
    esac
    for target in slave peer; do
        case_dir="$run_dir/$rate-$target"
        mkdir -p "$case_dir"
        if [[ "$target" == slave ]]; then
            source_dir="$repo_dir/firmware/slave_proto"
            board=hpm6300evk
            config_args=("-DSLV_MODE=$mode")
        else
            source_dir="$repo_dir/tools/slave_test"
            board=hpm6750evkmini
            config_args=("-DBUS_MODE=$mode" "-DROUNDS=$rounds" "-DPEER_PULLUP=$peer_pullup")
        fi
        echo "Building $rate-$target ($board, $mode)"
        if ! cmake -S "$source_dir" -B "$case_dir" -GNinja \
            "-DBOARD=$board" -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release \
            -DCMAKE_EXPORT_COMPILE_COMMANDS=ON "${config_args[@]}" \
            > "$case_dir/configure.log" 2>&1; then
            cat "$case_dir/configure.log" >&2
            die "configure failed: $rate-$target (evidence: $run_dir)"
        fi
        if ! cmake --build "$case_dir" --parallel "$jobs" > "$case_dir/build.log" 2>&1; then
            cat "$case_dir/build.log" >&2
            die "build failed: $rate-$target (evidence: $run_dir)"
        fi
        elf="$case_dir/output/demo.elf"
        [[ -f "$elf" ]] || die "ELF missing after successful build: $elf"
        "$nm" -S --defined-only "$elf" > "$case_dir/symbols.txt"
        "${hash_cmd[@]}" "$elf" > "$case_dir/elf-sha256.txt"
        printf '%s\t%s\t%s\t%s\t%s\tBUILT\tPENDING_HARDWARE\n' \
            "$rate-$target" "$board" "$mode" "$rounds" "$peer_pullup" >> "$run_dir/summary.tsv"
    done
done

printf 'All six release targets built. Hardware tests remain pending.\nSummary: %s/summary.tsv\n' "$run_dir"
