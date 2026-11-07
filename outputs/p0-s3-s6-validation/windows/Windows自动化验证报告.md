# S3–S6 Windows 自动化验证

日期：2026-11-07。当前 dirty 工作区的 x64 Windows 产物已完成跨编译，并在
ARM64 Windows 11 VM 的 x64 兼容环境执行。Rust 277/277、Python 18/18、
offline CLI 51/51 通过。USB 未连接，保持 BLOCKED；没有硬件通过项。
GUI 交互验证由主任务另行执行，结果不包含在本报告中。

## 环境与产物关联

- 构建：macOS/aarch64，rustc 1.96.0，cargo-xwin 0.23.1，
  `--release --workspace --locked --target x86_64-pc-windows-msvc`。
- 运行：Windows 11 Pro 10.0.26200，ARM64，UserInteractive=true，Session=1。
  guest 未安装 cargo/rustc/MSVC，本轮没有 Windows 原生编译。
- 7 个 EXE 均校验 PE Machine=0x8664，复制后、运行后 SHA-256 一致。
- `bundle/source-manifest.json` 保存 163 个 tracked/本任务新增文件的内容指纹；
  `bundle/binaries.json` 关联 4 个 Rust test EXE 与 3 个 app EXE。
  跨编译结束时全部源码与快照一致。测试结束时仅
  `docs/control-validation.md` 被主任务更新，Host 源码及测试输入没有漂移。
- 复用了原验证目录的 xwin 安装及 SDK cache；本轮构建 target、复制源码、
  脚本和证据均在新的隔离目录中，没有替换此前验证证据。

## 结果

| 项目 | 实际结果 | 证据 |
| --- | --- | --- |
| Windows workspace release | PASS | `cross-build.log` |
| Windows 全 test EXE 构建 | PASS | `cross-tests.jsonl` |
| usb-speed Rust tests | 7/7 PASS | `evidence/rust-usb_speed-cf9e1f83abd5a25f.log` |
| usb-i2c library tests | 80/80 PASS | `evidence/rust-usb_i2c-506d1d840cf5e26a.log` |
| usb-i2c CLI tests | 8/8 PASS | `evidence/rust-usb_i2c-0d3fd7d2dc08d298.log` |
| GUI Rust tests | 182/182 PASS | `evidence/rust-usb_i2c_gui-0fcc3a874534fb13.log` |
| Python adapter/runner | 18/18 PASS | `evidence/python-adapter.log` |
| offline CLI | 51/51 MODEL_PASS | `evidence/offline.json` |
| USB smoke | BLOCKED，exit 2，零设备 | `evidence/usb-smoke.json` |
| 临时隔离清理 | PASS | `evidence/cleanup.json` |

所有 Rust tests 的 failed/ignored/measured/filtered 均为 0。USB smoke 中
`usb_protocol_pass=1` 仅表示枚举调用正常返回，`blocked=1`，`hardware_pass=0`；
suite 将“缺设备正确报告 BLOCKED”作为软件检查通过，不把它计作设备功能通过。
物理频率、并发、Slave ISR/FIFO、recovery 波形、电气前端和 WinUSB 实机连接
均未验证。VM 的 x64 兼容运行也不等同于 x64 物理机验证。

## GUI 交接与清理

guest 隔离目录：
`C:\Users\zhaojianzhi\AppData\Local\usb-i2c-validation\s3-s6-20261107`

GUI：`bin\usb-i2c-gui.exe`

SHA-256：`3c5aecd895d3d5232cf929686f2a9062fd335c6dd3b16c4bf155df81545d8214`

CLI：`bin\usb-i2c.exe`

SHA-256：`1549cf61adc97a5877679a787816e652c58afade3aee4f0c34a1dd74463af1d9`

自动化脚本 `run_ps.py`、`run-suite.ps1`、`windows_suite.py` 保留在本目录；
`run-suite.ps1` 拒绝覆盖已存在的 guest run，重新运行应指定新的隔离目录。
测试通过临时 `R:` subst 映射重建编译时 `CARGO_MANIFEST_DIR` 源树副本，
并使用隔离 APPDATA；结束后已删除映射并恢复相关环境变量。

本子任务没有启动/关闭 GUI app。VM 的 pause-idle 原值为 on，自动化期间设为
off，交接时按主任务要求保持 off；主任务完成 GUI 后负责恢复。没有停止 VM，
没有修改生产源码或提交 Git。
