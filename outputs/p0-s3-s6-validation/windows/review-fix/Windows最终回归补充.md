# Windows 最终回归补充

2026-11-07：两处 GUI review 修复后的最终组合在 ARM64 Windows 11 的 x64
兼容环境运行 Rust **279/279 PASS**：usb-speed 7、usb-i2c library 80、CLI 8、
GUI 184；failed/ignored/measured/filtered 均为 0。

本次只重建 GUI app 与 GUI test EXE，覆盖 `worker.rs` 的未知结果提示，以及
`i2c/control.rs` 的 frontend SET-only 能力处理。其余三个 Rust test EXE 使用
首轮相同 SHA-256，相关源码与测试输入未改变，并在最终组合中重新运行。

Python 18/18 与 offline CLI 51/51 没有重复运行：其源码、CLI EXE 和已有证据
哈希保持一致，关联保存于 `bundle/final-expectations.json` 和
`evidence/suite-summary-final.json`。USB 保持首轮零设备 BLOCKED，
`hardware_pass=0`。本报告不包含 GUI 交互或真实硬件通过结论。

首轮 277 项报告、日志、源码快照和二进制均原样保留。仅 worker 修复时的中间
构建保存在 `worker-only-build/`，标为 SUPERSEDED，未在 Windows 执行；
最终完整源码快照位于本目录的 `bundle/source/`，不是旧快照冒充新构建输入。
最终构建和测试结束时，没有 Host 源码漂移。

最终 GUI：
`C:\Users\zhaojianzhi\AppData\Local\usb-i2c-validation\s3-s6-20261107\fixed\bin\usb-i2c-gui.exe`

SHA-256：`19005a5d952967f5774970090317780fc55bfc6fe0e118fe00a8fb2a753de38f`

最终 GUI test SHA-256：
`41087c1ae2f1d7e8fca678ed093aab7e29f4ed821b928379cb53c96a84f04ac9`

逐项日志：`evidence/rust-final-*.log`；机器汇总：
`evidence/suite-summary-final.json`；初版证据哈希及临时 subst/环境清理复核：
`final-verification.json`。本子任务没有操作旧 GUI 进程或 VM 设置；新版 app
交由主任务启动进行交互验证。
