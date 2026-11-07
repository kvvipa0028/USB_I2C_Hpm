# 本地 Windows 虚拟机验证记录

验证日期：2026-11-06–07（Asia/Shanghai）。本报告仅覆盖本次 Windows VM 验证；源码版本与产物关联见原始测试汇总和修复产物元数据。

## 结论

本地 Windows 11 VM 内的自动化测试、CLI 离线验证及本次选定的 GUI 离线交互验收已完成。原产物通过 **232 项 Rust 测试、9 项 Python adapter 测试、23 项 CLI 离线用例**；发现并修复批次页横向裁切后，新 GUI 产物通过 **18 项 I2C 针对性测试**，并完成双端口数据隔离、单端 NACK、长度边界、循环断连、重连不重放和配置清理的交互验收。

上述两轮测试对应不同产物，计数分别保留，不能合并成“新产物全量通过”。USB bridge 未接入 VM，USB 实机、legacy 固件及物理总线验收仍未完成。机器可读结论见 [gui-summary.json](evidence/gui-summary.json)。

## 环境与证据链

- Parallels Windows 11 专业版 ARM64，系统版本 10.0.26200，当前登录用户 SessionId=1、UserInteractive=True。
- 应用和 Rust 测试程序为 `x86_64-pc-windows-msvc`、PE AMD64（0x8664），在 Windows ARM 的 x64 兼容层执行；不是 ARM64 原生应用或 x64 物理机验证。
- 编译发生在 macOS，使用 cargo-xwin 交叉构建 release/test 程序；测试和 GUI 交互发生在本地 VM。不能据此声称已在该 VM 原生使用 Rust/MSVC 编译。
- GUI 使用经授权的 VM 内部 UIAutomation/输入模拟操作当前用户桌面。22 份快照包含动作、UIA 状态和截图；它们不是 22 个独立用例。
- 原源码压缩包 SHA-256 与 [environment.json](evidence/environment.json) 一致；原 7 个 EXE 的 SHA-256 与 [suite-summary.json](evidence/suite-summary.json) 一致，PE Machine 均为 AMD64。
- 修复后 GUI SHA-256：`8aeadc382caed18b4baec68184a734e801c0649c542ecb33eb06434502895d84`。修复测试程序 SHA-256：`8e184e50872bbff33104bf609e37dca8a45f89e7e98d4487e16f320d4596bac2`。
- 修复补丁 SHA-256：`0caf3ab6fe5efeeb0242960f3d0509fb6f30864b43cb63fbaab393ba8b4e3e0f`；已与 [source.patch](fixed/source.patch)、[GUI 元数据](evidence/gui-fixed-binary.json) 和 [针对性测试元数据](evidence/fixed-tests-summary.json) 交叉核对。`gui-06` 至 `gui-22` 直接记录相同的新 GUI 哈希；`gui-05` 通过 PID 与修复元数据关联。
- VM 工作目录：`C:\Users\zhaojianzhi\AppData\Local\usb-i2c-validation\run-20261106-sogVXD`。

## 自动化结果

| 产物与范围 | 实际结果 | 原始证据 |
| --- | --- | --- |
| 原 usb-i2c 共用库 | PASS，55/55，0 failed | [日志](evidence/rust-usb_i2c-506d1d840cf5e26a.log) |
| 原 usb-i2c CLI 单测 | PASS，3/3，0 failed | [日志](evidence/rust-usb_i2c-0d3fd7d2dc08d298.log) |
| 原 GUI 逻辑与无显示测试 | PASS，167/167，0 failed | [日志](evidence/rust-usb_i2c_gui-0fcc3a874534fb13.log) |
| 原 usb-speed | PASS，7/7，0 failed | [日志](evidence/rust-usb_speed-cf9e1f83abd5a25f.log) |
| Python CLI adapter | PASS，9/9 | [日志](evidence/python-adapter.log) |
| Windows CLI 离线端到端 | MODEL_PASS，23/23，retry_count=0、hardware_pass=0 | [报告](evidence/offline.json) |
| 修复后 GUI I2C 模块 | PASS，18/18，0 failed、0 ignored、150 filtered out；过滤器 `i2c::` | [日志，UTF-16](evidence/fixed-i2c-tests.log)、[元数据](evidence/fixed-tests-summary.json) |
| USB 冒烟 | BLOCKED，没有 USB bridge；退出码 2 符合无设备约定 | [报告](evidence/usb-smoke.json) |

原 Rust 合计 232 项；修复后针对性 18 项单列。`usb-smoke.json` 中 `usb_protocol_pass=1` 只表示 `list` 成功返回 `devices=[]`，不表示 WinUSB 实机已枚举成功，也不表示 PING/I2C 已通过。`offline.json` 的 `revision=null` 由外层 suite 与 EXE 哈希补充关联，不能单独当作完整版本证据。

## GUI 交互验收

| 检查 | 输入和观察结果 | 证据 |
| --- | --- | --- |
| 离线连接与刷新 | 显示 `OFFLINE-MODEL`、2 路端口、10000–1000000 Hz、单次读写各最多 256 B；刷新保持离线连接；重复连接按钮禁用 | [01](evidence/gui-01-offline.json)、[02](evidence/gui-02-master.json)、[05](evidence/gui-05-fixed-offline.json) |
| CH0/CH1 独立数据 | 地址均 0x50、偏移 0x10；CH0 写 `10 A1 A2` 后读回 `a1 a2`，CH1 写 `10 B1 B2` 后读回 `b1 b2`；本步骤两路请求速率均 100000 Hz | [07](evidence/gui-07-ch0-write.json)、[08](evidence/gui-08-ch0-readback.json)、[10](evidence/gui-10-ch1-readback.json) |
| 批次分别返回结果 | 两端写后读 HEX=`10`、长度=2，分别显示 `离线 CH0 ... a1 a2`、`离线 CH1 ... b1 b2` | [12 JSON](evidence/gui-12-batch-independent.json)、[截图](evidence/gui-12-batch-independent.png) |
| 单端 NACK | CH0 地址改 0x51：`CH0: Nack (hpm_stat 2003)`；CH1 仍返回 `b1 b2`，保持连接；底栏提示部分端口失败、未自动重试 | [13 JSON](evidence/gui-13-batch-nack.json)、[截图](evidence/gui-13-batch-nack.png) |
| 批次长度边界 | CH0 读长 241：显示最多 240 B、提交按钮禁用。改为 240 后可提交，返回恰好 240 B：`a1 a2` 后 238 个 00 | [14](evidence/gui-14-batch-overlimit.json)、[15](evidence/gui-15-batch-limit240.json) |
| 循环及主动断开 | CH1 写后读、默认循环周期 100 ms；活动快照计数 19。循环继续至主动断开时计数 159，显示循环已停止，读写按钮禁用 | [17](evidence/gui-17-loop-active.json)、[18](evidence/gui-18-disconnected.json) |
| 重连不自动重放 | 断开后计数 159；重连并等待 1200 ms 后仍为 159，再等待 1500 ms 后仍为 159。两次重连后快照相隔约 26.266 s，结果仍为历史 #159 | [19](evidence/gui-19-reconnected.json)、[20](evidence/gui-20-no-replay.json) |
| 重连清理配置与模型 | 主机恢复 CH0/100000 Hz，显示“尚未保存”、撤销修改禁用；旧 #159 记录保留“离线模型 · CH1”来源。显式改为 400000 Hz 后请求，新增恰好 #160，偏移 0x10 读回 `00 00`，证明新模型数据重置；实际频率未测量 | [19](evidence/gui-19-reconnected.json)、[20](evidence/gui-20-no-replay.json)、[21 JSON](evidence/gui-21-new-session-read.json)、[截图](evidence/gui-21-new-session-read.png) |
| 批次结果清理 | 重连后批次页无旧批次结果，两端批次速率恢复 100000 Hz | [22](evidence/gui-22-batch-cleared.json) |
| 页签稳定 | 主机、批次、从机再回主机，三页签相对窗口矩形始终为 `(909,104,51,26)`、`(962,104,51,26)`、`(1015,104,88,26)` | [06](evidence/gui-06-fixed-master.json)、[11](evidence/gui-11-fixed-batch-layout.json)、[16](evidence/gui-16-slave-header.json)、[17](evidence/gui-17-loop-active.json) |

原生 GUI 的部分文字由绘制层呈现，UIA 不一定包含全部文本；因此同时核对 JSON 与截图。`gui-08`、`gui-13` 是操作后的状态快照，`actions=[]` 不代表没有执行；其输入值、结果和截图可证明上述观察，但不作为完整动作回放记录。仅出现预期关键词不等于用例通过，本报告使用结果数据、计数与按钮状态作为判据。

## 已发现并修复的问题

初版批次页的操作选择控件占满横向空间，导致地址、SCL 及后续字段超出窗口。原生 UIA 显示 CH0 地址 x=1211、CH1 地址 x=1361，全部 8 个 Edit 控件的右边缘越界；[原始失败截图](evidence/gui-03-batch.png) 和 [原始 JSON](evidence/gui-03-batch.json) 已保留。

局部修复 `host/crates/gui/src/i2c/batch.rs` 后，[修复截图](evidence/gui-11-fixed-batch-layout.png) 中地址/SCL 可见，8 个 Edit 控件均在窗口内，且后续输入和批次操作成功。修复前外框为 1212×834，修复后为 1216×839：均为约 1200 px 宽窗口，宽差 4 px、高差 5 px，不能描述为截图像素完全同尺寸。新增布局测试包含在修复后的 18 项针对性测试中。

## 证据格式与未验证部分

`gui-01` 至 `gui-03` 各有 4 个匿名 Pane 的 UIA 空矩形 `[Infinity, Infinity, -Infinity, -Infinity]`，原始文件均保留。其严格 JSON 副本位于 [normalized](evidence/normalized/manifest.json)：只将非有限矩形改为 `null`，保留 `rect_raw` 字符串和源 SHA-256，所有有限坐标不变；不会通过截断坐标掩盖裁切问题。`gui-04` 以后已由采集脚本输出有限矩形或 `null`。摘要和规范化文件均可用严格 JSON 解析器读取。

以下边界仍保留为未验证：

- 无 USB 设备透传或 WinUSB 实机 PING/I2C；空设备列表只证明无设备路径正常。
- 无真实 legacy 固件连接及旧/新 host、旧/新 firmware 兼容组合。legacy 控件与快照保护只有自动化逻辑证据。
- 主动点击断开不等价于在途 USB 失联，未验证实机写入结果未知、物理写入是否重放。
- 离线模型不模拟实际 SCL、时钟延展、FIFO/ACK、电气故障或双路物理并发；400000 Hz 只验证参数与展示，不能作为实际 400 kHz 波形证据。
- 未验证原生 ARM64 构建、Windows x64 物理机、该 VM 本机编译、安装包/签名/升级及多 DPI/多屏组合。当前原生截图仅覆盖本次桌面和窗口条件。

原始测试、原始失败及修复后证据均分开保留；本报告没有将 S3–S6 或上板工作标记为完成。
