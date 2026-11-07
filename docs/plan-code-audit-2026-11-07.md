# USB_I2C_Hpm 计划与代码偏差审计

归档说明（2026-11-07 20:01 北京时间，2026-11-07T12:01:46Z）：本文是审计基线 `c41d6ab6273fe1d48eaf03cab384ca138009e2e9` 的原报告归档，不是对最新 `main` 的重新审计，也不声称最新全仓穷尽审计。审计结束时 `origin/main` 已是 `26723fe4497afee8df90ece7388d0b57097ff8f1`；该提交新增的验证报告和忽略规则没有进入下面的 47 条矩阵。本文件是事后放入 `docs/` 的归档，同样未纳入该矩阵。47 条、固定 SHA 行链接、测试结果和验证边界保持原样。

- 审计基线，即审计开始时的 `origin/main`：`c41d6ab6273fe1d48eaf03cab384ca138009e2e9`
- 开始时间：2026-11-07 19:39:39 北京时间（2026-11-07T11:39:39Z）。当时 `git fetch origin main` 后该 SHA 与已知的 PR #9 + PR #10 合入点一致
- 基线提交时间：2026-11-07 19:08:54 北京时间（2026-11-07T19:08:54+08:00），说明：`Merge pull request #10 from kvvipa0028/codex/p0-s3-s6-controls`
- 本报告全部源码与文档行号都指向审计基线 `c41d6ab`。行链接基址：
  `https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/`
- 工作树：审计期间 detached HEAD 停在该 SHA，结束时 `git status --porcelain` 为空。审计本身未改仓库源码、SDK 或其他既有计划文档。
- 结束复核：2026-11-07 19:49:38 北京时间（2026-11-07T11:49:38Z）再次 `git fetch origin main`，`origin/main` 已变为 `26723fe4497afee8df90ece7388d0b57097ff8f1`（提交时间 2026-11-07 19:46:10 北京时间，2026-11-07T19:46:10+08:00，`Merge validation reports and output ignore rules into main`）。相对锁定 SHA 只有 `.gitignore` 与 5 份 Markdown，无产品源码差异。下文结论针对锁定 SHA；第 5 节说明这次前移。本归档文件不在该差异内，也不在 47 条矩阵内。

## 1. 结论与偏差规模

锁定 SHA 上，S0–S6 控制面的协议、双端口状态机、主机 API、CLI 用例编号、GUI 的 I2C 调用链、离线双模型、HIL 骨架、native 测试和 UDS P1 宿主/模拟/v1 manifest 解析，与 2026-11-07 的协议、验收和 README 状态表大体一致。偏差集中在四类，而不是“产品固件仍只有 PING/I2C”：

1. 仍生效的单写者约束在三处产品/对端路径上未满足（F1 USB 槽、F2 I2C 端口、F3 siggen）。这是双写者仍在，不是已实现的 UAF，也不是未授权单 trap 设计已经落地。
2. 10 处当前文档互相矛盾或落后于 `c41d6ab` 源码。最严重的是 UDS 决策状态、旧提案基线、UI 合并说明里的“扫描/恢复仍是演示”。
3. 7 项计划中的能力有代码但验收或集成未闭合：400 kHz/1 MHz 与 10 分钟、双端口物理重叠、表服务时限、波形、真实 USB/Windows/热插拔、`i2c_proto` 频率输出、siggen 实板。
4. 7 项仍是计划中的后续交付：C/Python SDK、Bootloader/签名/防克隆实现、产品电气与长稳、真实 ISO-TP/UDS 刷写、P1 扩展、安装包/签名、量产 VID。当前状态文档已把它们标成未做或未授权实现，不把它们算作 PR #10 交付失败。

可复核分母是 **47** 条互斥条目。其中 **16** 条与现行有效要求吻合，**31** 条属于其余五类。不计算总体完成百分比。

未纳入交付缺陷的事项：纸面架构 C1 子队列公平、C2 过期定时反复唤醒、C3 PINNED 租约引用复用 op；未授权且未完成全约束证明的单硬件 trap / 有界 USB 设计；DMA 停止与 USB 恢复时限未证；HPM SDK 部署 SHA 未锁定。PR #10 合入不等于设计、审查或硬件验收通过。这些只在第 4、5 节作为排除项出现。

## 2. 统计口径与分母

### 2.1 文档优先关系

现行要求只取尚未被后文撤回的句子。

| 优先级 | 文档 | 用途 |
| --- | --- | --- |
| 1 | `docs/control-protocol-v1.md`、`docs/usb-protocol.md`（2026-11-07 更新）、`docs/control-validation.md`、`docs/control-cli-test-plan.md`、`README.md` 状态表、`host/crates/gui/UI_MERGE_NOTES.md` 行为表 | 当前契约与已声明状态 |
| 2 | `docs/test-plan.md`、`docs/s1-bench-readiness.md`、`docs/p0-development-order.md` 未关闭出口、`docs/pr10-status-and-handoff.md` 与 `docs/pr10-handoff/*` | 仍开放的验收与单写者/电气约束 |
| 3 | `PRD.md`、`DEV_PLAN.md`、`功能清单_UTA0301对标.xlsx` | 草稿与对标。后文未撤回时，记“计划中未做”；后文已改口时，旧句只记文档过时 |
| 历史 | `docs/uds-bootloader-plugin-proposal.md` 中已被同文 §4.4/§10 改写的基线句、`UI_MERGE_NOTES.md` 结构节里已被行为表改写的句子、`p0-development-order.md` 中“尚无 CH1 映射” | 不单独构成当前代码义务 |

`功能清单_UTA0301对标.xlsx` 日期 2026-10-28，三张表是目标规格，没有实现状态列，不能按勾选判完成。

### 2.2 互斥主类

每条只进一个主类。风险标签不另计一条。

| 主类 | 数量 | ID |
| --- | ---: | --- |
| 已实现吻合 | 16 | M01–M16 |
| 部分实现 | 7 | P01–P07 |
| 未实现但计划中 | 7 | N01–N07 |
| 实现违背当前约束 | 3 | V01–V03 |
| 文档过时或相互矛盾 | 10 | D01–D10 |
| 证据不足 | 4 | E01–E04 |
| 分母 | 47 |  |

风险标签（不计入上表）：`HW` 需实板或仪器；`DOC` 下一步会被旧文档带偏；`OWN` 单写者；`ENV` 本机工具链不足以复现历史主机测试。

### 2.3 去重与排除

- 前端电压：协议要求 `0x19/0x1A` 返回 Unsupported 且不得伪造电压，记 M05。产品前端硬件、ESD、电流和 24 小时另记 N03。
- UDS：P1 宿主、Mock、v1 解析器记 M14。真实 ISO-TP、固件刷写、ODX 记 N04。
- 时序计算器：`i2c_timing.c` 已存在，旧“待实现”句子记 D06；`i2c_proto` 仍无频率输出模式记 P06。
- CH1 映射：文档互相矛盾记 D07；SDK 宏未在本机重核记 E02。
- 安装包与设备验收分开：真实 GUI/Windows 驱动/热插拔记 P05；安装包与代码签名记 N06。
- 第二批 SPI/CAN/LIN/UART/PWM/GPIO/ADC 固件不单列。PRD §4.2 与 DEV_PLAN §8.3 标为 V1.x/V2，README 把页面标成本地演示。页面范围与旧规格的冲突只记 D09。
- C1–C3 与单 trap 不进分母。

## 3. 逐项矩阵

链接均固定到 `c41d6ab6273fe1d48eaf03cab384ca138009e2e9`。

### 3.1 已实现吻合（16）

#### M01 旧版 PING / CAPABILITIES / 单路 I2C

- 有效要求：[usb-protocol.md:9-24](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/usb-protocol.md#L9-L24)，命令 0x01、0x02、0x10，状态 0–4。
- 实现：[bridge_protocol.c:15-20](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/bridge_protocol.c#L15-L20)、[bridge_protocol.c:67-88](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/bridge_protocol.c#L67-L88)、[proto.rs](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/usb-i2c/src/proto.rs)。
- 测试：本机 `firmware/tests` 的 `protocol` 通过（见第 5 节）。历史板级日志不在本 SHA。
- 主类：已实现吻合。影响：旧主机仍可用。优先级：维持。建议：保持 0x10 的 0–4 状态兼容。

#### M02 能力块与严格降级

- 有效要求：[usb-protocol.md:24-44](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/usb-protocol.md#L24-L44)，20 字节、version 1、max 256、batch 240、10 kHz–1 MHz、默认 100 kHz；只有精确 BadRequest 才允许降到旧能力。
- 实现：[bridge_protocol.c:67-77](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/bridge_protocol.c#L67-L77)，`out[2]=7`，长度 256/240，速率 100000/10000/1000000。
- 测试：`protocol` native 通过。未在板上读回描述符。
- 主类：已实现吻合。影响：主机可拒绝旧固件。优先级：维持。建议：实板只补读回，不改字段。

#### M03 端口命令、批量预检与逐项结果

- 有效要求：[control-protocol-v1.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/control-protocol-v1.md) 的 0x11/0x12；任一项预检失败则两条总线都不 START。
- 实现：[bridge_protocol.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/bridge_protocol.c) 的 `transfers_start` 先校验再启动；[i2c_port.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/i2c_port.c) 注释写明两端口都准备好才 START。
- 测试：`protocol`、`port_state_machine` 通过。物理重叠见 P02。
- 主类：已实现吻合。影响：软件契约已闭合。优先级：维持。建议：板上用双逻辑分析仪确认，不把软件预检写成物理同时性。

#### M04 控制能力与 0x13–0x18

- 有效要求：[control-protocol-v1.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/control-protocol-v1.md)，0x03 为 48 字节，features `0x9F`，表 256、服务 2000 µs、stretch 1000–100000 默认 25000、事务 1000–2000000 默认 500000、控制截止 1500000。
- 实现：`bridge_protocol.c` 的 control caps 写入上述常数；0x13–0x18 分发到端口/表/探测/恢复。
- 测试：`protocol` 与 `port_state_machine` 通过。2000 µs 是否真正返回状态 9 见 P03。
- 主类：已实现吻合。影响：主机与固件数字契约一致。优先级：维持。建议：把“广告值”和“已测 WCET”在报告里继续分开。

#### M05 前端命令返回 Unsupported

- 有效要求：0x19/0x1A 返回 Unsupported，不伪造电压。
- 实现：`bridge_protocol.c` 对 0x19/0x1A 返回 `BRIDGE_UNSUPPORTED`。GUI 前端页不发真实电压。
- 测试：协议 native 覆盖该状态。无前端板。
- 主类：已实现吻合。影响：主机不会把缺硬件显示成有效电压。优先级：维持。建议：硬件设计另行按 N03 做，不改这个失败语义。

#### M06 从机表、CAS 与代数

- 有效要求：256 字节活动表/镜像、短临界区外拷贝、外部写与 PC 提交各增加 generation、偏移写不增加、外部 EEPROM 在 0xFF 回绕、PC 片段不回绕。
- 实现：[i2c_table.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/common/i2c_table.c)、[host/crates/gui/src/i2c/slave.rs:107-158](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/src/i2c/slave.rs#L107-L158) 的 `TableRead`/`TableWrite`。
- 测试：`common` native 通过。外部主设备对产品固件的板级表测试未做；T1.9 只覆盖独立 `slave_proto`。
- 主类：已实现吻合。影响：软件线性化已在。优先级：P1。建议：用外部主设备对 `usb_i2c` 从机做一轮，不把 T1.9 直接当作产品证据。

#### M07 时序计算器 ±5%

- 有效要求：HPM6300 SETUP/TPM 方程，只接受 ±5% 以内的解；计算不等于波形合格。
- 实现：[i2c_timing.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/common/i2c_timing.c)。
- 测试：`common` native 通过。波形见 P04。
- 主类：已实现吻合。影响：频率选择有计算门。优先级：维持。建议：旧文档“待实现”按 D06 改，不重复实现一份。

#### M08 恢复 9 个时钟与开漏释放

- 有效要求：9 个 SCL 后 STOP；GPIO 锁存 0，输出拉低，输入释放，不主动拉高；总超时 1.5 s。
- 实现：[i2c_recovery.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/common/i2c_recovery.c)、[i2c_board.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/boards/evk/i2c_board.c)。
- 测试：`common` native 通过。示波器未见。
- 主类：已实现吻合（软件状态机）。影响：恢复命令有明确失败码 0–4。优先级：P1，`HW`。建议：S1 只补波形，不把软件边沿 5 µs 写成已测电气。

#### M09 实时截止步进

- 有效要求：总事务截止与 SCL 低电平截止；字节有进展才重置低电平区间。
- 实现：[i2c_deadline.h:1-19](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/common/i2c_deadline.h#L1-L19)，`master_poll` 使用配置里的事务/拉伸超时。
- 测试：端口状态机 native 通过。拉伸示波器见 P04。
- 主类：已实现吻合。影响：软件不再用固定轮询次数冒充时间。优先级：维持。建议：实板用可控时钟拉伸仪核对。

#### M10 显式离线、双模型、USB 失败不降级

- 有效要求：无设备或 USB 损坏是错误；离线只在显式选择时进入；外部 EEPROM 与从机表分开。
- 实现：[device.rs open_usb](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/usb-i2c/src/device.rs#L78-L124) 注释写明不回落离线；`offline()` 建 `external_tables` 与 `slave_tables` 各两份；[bridge.rs Session::fail](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/usb-i2c/src/bridge.rs#L170-L199) 丢弃传输且不重放。
- 测试：本机未跑 Rust。代码路径与 CLI 计划一致。历史 51/51 日志不在本 SHA，见 E03。
- 主类：已实现吻合（源码契约）。影响：离线结果不会被当成 USB 成功。优先级：维持。建议：Rust 测试留到 1.95 工具链，不把缺跑记成功能缺失。

#### M11 CLI 面与静态用例数

- 有效要求：[control-cli-test-plan.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/control-cli-test-plan.md) 的 8 个 CLI 测试、18 个 Python 测试、51 个离线用例。
- 实现：[main.rs](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/usb-i2c/src/main.rs) 含 8 个 `#[test]`；`tools/hil/test_runner.py` 18 个测试方法；`build_cases()` 23 + `build_control_cases()` 28 = 51。
- 测试：本机 Python 18/18 通过。Rust 8 个未跑。
- 主类：已实现吻合。影响：离线矩阵编号可复核。优先级：维持。建议：硬件用例名称与 `control-validation.md` 的细微差异留在报告里，不改编号除非双方同时改。

#### M12 GUI 的 I2C 工作线程与扫描取消

- 有效要求：传输、批量、控制、连接走工作线程；扫描取消是原子标志，不是排队任务。
- 实现：`Job::` 只出现在 `host/crates/gui/src/worker.rs` 与 `app.rs`；扫描取消为 `Arc<AtomicBool>`。
- 测试：本机未开窗口、未跑 GUI 测试。调用图已核。
- 主类：已实现吻合（调用链）。影响：I2C 页不会绕过 `Device`。优先级：P1。建议：真实 USB 点击留到有设备的机器，见 P05。

#### M13 非 I2C 页不发设备任务

- 有效要求：README 与行为表把 SPI/CAN/LIN/UART/PWM/GPIO/ADC/ADS1015/EEPROM/Flash 标为本地演示。
- 实现：抽查 `spi.rs`、`can.rs` 无 `usb_i2c`、`Job::`、`ControlRequest`。EEPROM 页写本地 `mem`。全仓 `Job::` 仅两处。
- 测试：静态调用图。未逐控件点击。
- 主类：已实现吻合。影响：演示页不会被误认成已发 USB。优先级：维持。建议：D09 只改文档，不删演示页，除非产品决定收窄界面。

#### M14 UDS P1 宿主、Mock 与仅解析的 v1 manifest

- 有效要求：P1 为宿主、`.udsmock`、manifest 解析；解析器不接入 Mock 与 UI 源选择。真实刷写未开始。
- 实现：`host/crates/gui/src/flow/api.rs`、`host.rs`、`mock.rs`、`manifest.rs`；`manifest.rs` 使用 serde。无 CAN/ISO-TP 固件命令。
- 测试：本机未跑 GUI 测试。源码存在，故不能报“UDS 完全缺失”。
- 主类：已实现吻合。影响：模拟诊断链路可继续用。优先级：维持。建议：真实刷写按 N04，不要把 P1 写成刷写已开始。

#### M15 HIL 硬件通过数为 0，USB 冒烟只读

- 有效要求：无设备时 `hardware_pass=0`；USB 模式只列待测硬件项，不发 I2C 写。
- 实现：`tools/hil/run.py`、`cases/offline.py`、`cases/control.py`、`adapters/cli.py`。
- 测试：本机 18 个 Python 测试通过，且不打开 USB、不编译 CLI。
- 主类：已实现吻合。影响：离线结果带有明确的硬件零通过。优先级：维持。建议：有设备时再打开写用例。

#### M16 CI 与文档描述一致

- 有效要求：[control-validation.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/control-validation.md) 描述的 host 工作流。
- 实现：[.github/workflows/host.yml](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/.github/workflows/host.yml)：Ubuntu 跑 firmware-native 与 siggen-native；macOS 与 Windows 跑 cargo test/clippy/release、Python 与离线 HIL。注释写明不声称硬件。Linux 主机 cargo 不是独立作业。
- 测试：未重跑 GitHub Actions。只核对工作流文本。
- 主类：已实现吻合。影响：CI 范围与文档一致。优先级：维持。建议：历史 CI 绿不替代本次验证。

### 3.2 部分实现（7）

#### P01 从机速率与 10 分钟未关闭

- 有效要求：[s1-bench-readiness.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/s1-bench-readiness.md)、[test-plan.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/test-plan.md)：产品路径 100 kHz/400 kHz/1 MHz，以及 10 分钟。
- 实现：产品桥可配置从机 100 k/400 k/1 M。独立 [slave_proto](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/slave_proto/src/main.c) 仍是另一固件。T1.9 记录仅为 100 kHz。
- 测试：无本机实板。仓库内无 400 kHz、1 MHz、10 分钟通过记录。
- 主类：部分实现。影响：不能把 T1.9 写成产品从机验收。优先级：P0，`HW`。建议：先在 `usb_i2c` 上做 400 kHz，再 1 MHz，最后 10 分钟。

#### P02 双端口物理重叠与故障隔离未测

- 有效要求：两端口可同时处于传输；一端口故障不重置另一控制器。
- 实现：[i2c_board.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/boards/evk/i2c_board.c) 给出 CH0=I2C0 PC13/PC14、CH1=I2C3 PC11/PC12；软件先启动再轮询。注释写明接插件冲突待测。故障路径只复位所选控制器。
- 测试：状态机 native 通过。无双通道波形。
- 主类：部分实现。影响：软件并发不等于引脚同时合法。优先级：P0，`HW`。建议：先确认接插件，再测重叠与单端复位。

#### P03 表服务 2000 µs 只记录最大值

- 有效要求：协议广告 `table_service_us=2000`；控制验收把 S3-SERVICE 留为未测。
- 实现：`i2c_table` 超时仍返回 OK，并更新 `g_bridge_table_service_us_max`。枚举有 `BRIDGE_SERVICE_DEADLINE`，固件不返回它。主机扫描在本地满 5 s 才合成 ServiceDeadline。
- 测试：native 未证明 2000 µs 截止。板上无 WCET。
- 主类：部分实现。影响：状态 9 目前不是固件表路径的真实返回。优先级：P0。建议：先决定“只广告设计值”还是“超时必须返回 9”，再改一处并补测试。

#### P04 速率、拉伸、扫描、恢复没有波形

- 有效要求：S1 要求频率、stretch、扫描和恢复的仪器证据。
- 实现：计算器、截止步进、扫描命令和恢复状态机都在。siggen 有对应命令。
- 测试：native 通过。无示波器记录。
- 主类：部分实现。影响：数字行为不能代替电气验收。优先级：P0，`HW`。建议：按 S1 清单逐项补波形，不新写协议。

#### P05 真实 GUI、Windows 驱动与热插拔未证

- 有效要求：WinUSB 免驱、GUI 真实点击、热插拔后会话作废且不重放。
- 实现：描述符在 [usb_desc.c:15-25](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/usb_desc.c#L15-L25)；GUI I2C 走 worker；失败丢弃会话见 M10。
- 测试：本机无板、无 Windows、无 GUI 窗口。锁定 SHA 内无本次交互日志。
- 主类：部分实现。影响：描述符存在不等于新机器免驱通过。优先级：P0，`HW`。建议：用未装驱动的 Windows 10/11 做一次枚举和拔插。

#### P06 `i2c_proto` 仍只有 100 kHz

- 有效要求：S1 频率模式需要六档输出。
- 实现：[firmware/i2c_proto/src/main.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/i2c_proto/src/main.c) 仍调用 SDK `board_init_i2c`，固定 100 kHz。产品桥的 `i2c_timing.c` 没有接到这个样例。
- 测试：无。
- 主类：部分实现。影响：旧样例不能用来出六档波形。优先级：P1。建议：若 S1 仍用它做激励，再接入计算器；产品桥本身不因此缺命令。

#### P07 siggen 软件在、实板未做

- 有效要求：对端固件提供从机、恢复和 CDC，供 EVK 验收。
- 实现：`tools/siggen/src/` 有命令、peer、CDC、恢复 GPIO。
- 测试：本机 GCC ASan/UBSan 5/5 通过。未烧 HPM6750EVKMini。
- 主类：部分实现。影响：对端可编译测试，不能当作已配对。优先级：P1，`HW`。建议：与 P01/P04 同一轮烧录。

### 3.3 未实现但计划中（7）

这些条目的当前状态文档已写明未做或未授权实现。它们是后续范围，不是本 SHA 的隐蔽回归。

#### N01 C API 与 Python SDK

- 有效要求：PRD FR-SDK-01/02，DEV_PLAN 对应任务。README 状态表写明未实现。
- 实现：`host/Cargo.toml` 无 `sdk-c`、`sdk-py`。`tools/hil/adapters/python_sdk.py` 不存在。
- 测试：无交付物可测。
- 主类：未实现但计划中。影响：第三方只能用 Rust/CLI。优先级：P1。建议：等协议验收稳定后再做绑定。

#### N02 Bootloader、签名与防克隆

- 有效要求：[firmware-update-design.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/firmware-update-design.md) 是设计；README 写明未实现。无 OTP 烧写授权。
- 实现：无 `firmware/bootloader`。现有 UID 只用于 USB 序列号。
- 测试：无。
- 主类：未实现但计划中。影响：现场升级仍靠 OpenOCD。优先级：P2。建议：先锁签名格式和失败回退，再写代码。

#### N03 产品电气、ESD、电流与 24 小时

- 有效要求：PRD 电气行与 S5/S10。仓库无 `hardware/`。
- 实现：无原理图、治具和记录。软件前端按 M05 返回失败。
- 测试：无。
- 主类：未实现但计划中。影响：不能做电平、ESD、≤200 mA 和长稳结论。优先级：P1，`HW`。建议：硬件设计单独立项，不从固件推断。

#### N04 真实 ISO-TP、UDS 刷写、ODX 与 CAN 固件

- 有效要求：[uds-flash-implementation-plan.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/uds-flash-implementation-plan.md)、[uds-isotp-firmware-plan.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/uds-isotp-firmware-plan.md) 写明计划、未开始、不授权实现。
- 实现：无对应固件命令。P1 已按 M14 存在。
- 测试：无真实刷写用例。
- 主类：未实现但计划中。影响：不能刷 ECU。优先级：P2。建议：D04 未关闭前不开始固件。

#### N05 P1 扩展

- 有效要求：10 位地址、24Cxx 工具、多命令序列。PRD 标 P1。
- 实现：10 位被拒绝。EEPROM 页是本地演示。无多步命令执行器。
- 测试：无。
- 主类：未实现但计划中。影响：当前主机只覆盖 7 位与单次传输。优先级：P2。建议：保持拒绝 10 位，直到协议有字段。

#### N06 Windows 安装包与代码签名

- 有效要求：DEV_PLAN 的发布步骤。
- 实现：无安装工程、无签名脚本。
- 测试：无。
- 主类：未实现但计划中。影响：不能按产品安装包交付。优先级：P2。建议：与免驱验收分开，见 P05。

#### N07 量产 VID/PID

- 有效要求：DEV_PLAN D10 待取得合规 ID。
- 实现：[usb_desc.c:15-18](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/usb_desc.c#L15-L18) 使用 0x1209:0x0001，注释写明不得出货。
- 测试：无。
- 主类：未实现但计划中。影响：当前 ID 与“不得出货”一致，不是写错 ID。优先级：P2。建议：取得 ID 前保持测试 ID 和注释。

### 3.4 实现违背当前约束（3）

单写者标准未放宽。下列是 ISR/回调与任务同时写同一状态，不是已经存在的 UAF 证明，也不是 C1–C3 或单 trap 已编码。

#### V01 F1：USB 槽双写

- 有效要求：[pr10-handoff/ownership.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/pr10-handoff/ownership.md)、[pr10-status-and-handoff.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/pr10-status-and-handoff.md)：一个可变状态只有一个写者。
- 实现：[usb_i2c.c:15-36](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/usb_i2c.c#L15-L36) 中 `usbd_event_handler` 写 `configured`、`rx_ready`、`rx_armed`、`epoch`；[usb_i2c.c:56-94](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/usb_i2c.c#L56-L94) 中 `usb_i2c_task` 写同组标志及 `tx_done`。
- 测试：`usb_slot` native 通过，只说明替身下的功能路径，不证明单写者。
- 主类：实现违背当前约束。影响：并发可见性仍靠 volatile 与时序巧合。优先级：P0，`OWN`。建议：先定唯一写者再改代码；不要直接实现未授权的单 trap。

#### V02 F2：I2C 端口双写

- 有效要求：同上，从机表、从机状态和 I2C 寄存器各有一个写者。
- 实现：[i2c_port.c:172-229](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/i2c_port.c#L172-L229) 的从机 ISR 写表和寄存器；[i2c_port.c:262-345](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/firmware/usb_i2c/src/i2c_port.c#L262-L345) 的任务写主状态和寄存器。短关中断只缩小窗口，不取消第二个写者。
- 测试：`port_state_machine` 通过，不覆盖双上下文。
- 主类：实现违背当前约束。影响：表与总线状态的并发规则未闭合。优先级：P0，`OWN`。建议：与 V01 同一所有权设计，分开提交。

#### V03 F3：siggen 双写

- 有效要求：对端同样受单写者约束。
- 实现：[siggen_peer.c:209](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/tools/siggen/src/siggen_peer.c#L209) 在接收路径写表；[siggen_peer.c:344-346](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/tools/siggen/src/siggen_peer.c#L344-L346) 在主循环 `prepare` 写表。[cdc_acm.c](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/tools/siggen/src/cdc_acm.c) 的 USB 回调与 `cdc_write` 都写 `configured`/`ep_tx_busy`。
- 测试：本机 siggen 5/5 通过，不证明单写者。
- 主类：实现违背当前约束。影响：对端验收固件与产品固件有同一类问题。优先级：P0，`OWN`。建议：不要在对端先引入另一套同步。

### 3.5 文档过时或相互矛盾（10）

#### D01 UDS 决策状态不一致

- 文档 A：[README.md:21](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/README.md#L21)、[UI_MERGE_NOTES.md:114](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/UI_MERGE_NOTES.md#L114) 写 D01–D07 全部待确认。
- 文档 B：提案 §10 与刷写计划 §2 写 D04 已确认，D01/D03/D05 部分确认，D02/D06/D07 开放。
- 代码：M14 的 P1 已在，真实刷写未开始。
- 主类：文档过时或相互矛盾。影响：下一步会把已确认的自研 HPM ISO-TP 通道当成未决。优先级：P0，`DOC`。建议：以提案 §10 为准改 README 和 UI 说明，并保留“未授权实现”。

#### D02 提案仍写协议只有 PING 与 I2C

- 文档：[uds-bootloader-plugin-proposal.md:41](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/uds-bootloader-plugin-proposal.md#L41) 仍写真实通信只有 PING 和 I2C。同文 [L348](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/uds-bootloader-plugin-proposal.md#L348) 已把 D04 标成已确认，后文也承认 v1 manifest。
- 代码：`bridge_protocol.c` 已有 0x02、0x03、0x10–0x1A。
- 主类：文档过时或相互矛盾。影响：读者会重做已经存在的控制命令。优先级：P0，`DOC`。建议：把该段标成 2026-11-06 基线或删除。

#### D03 刷写/ISO-TP 文首与正文阶段词不一致

- 文档：两份计划文首写“未开始实现”。正文阶段 A–D 也确实未开始。P1 与 v1 解析已完成。
- 代码：M14 存在，N04 不存在。
- 主类：文档过时或相互矛盾。影响：“未开始”会被理解成连 P1 都没有。优先级：P1，`DOC`。建议：文首改成“真实刷写未开始；P1 已合入”。

#### D04 UI 结构节与行为表冲突

- 文档：行为表写 I2C 控制走真实 USB/离线。[UI_MERGE_NOTES.md:153](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/UI_MERGE_NOTES.md#L153) 写 `worker.rs` 未改，[L164](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/UI_MERGE_NOTES.md#L164) 写扫描/恢复仍是演示。
- 代码：worker 已有 Control/Scan/Recover；`slave.rs` 有 CAS 下发。
- 主类：文档过时或相互矛盾。影响：会把已接通的扫描、恢复和表下发当成演示。优先级：P0，`DOC`。建议：删除结构节的过时句，保留行为表。

#### D05 Cargo.toml“未改”与 serde 已加入

- 文档：[UI_MERGE_NOTES.md:125](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/UI_MERGE_NOTES.md#L125) 写 Cargo.toml 未改、没有新增依赖；[L133](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/host/crates/gui/UI_MERGE_NOTES.md#L133) 写本 PR 新增 `serde`、`serde_json`。
- 代码：`host/crates/gui/Cargo.toml` 依赖 serde、serde_json。
- 主类：文档过时或相互矛盾。影响：小，但会误判依赖差分。优先级：P2，`DOC`。建议：删前一句。

#### D06 S1 仍写时序入口待实现

- 文档：[s1-bench-readiness.md:23](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/s1-bench-readiness.md#L23) 写算法尚未完成；[L127](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/s1-bench-readiness.md#L127) 写需要先实现受检计算入口。其中针对 `i2c_proto` 仍只有 100 kHz 的句子成立。
- 代码：`i2c_timing.c` 已在产品路径。`i2c_proto` 见 P06。
- 主类：文档过时或相互矛盾。影响：会再写一个计算器。优先级：P1，`DOC`。建议：句子限定到 `i2c_proto`。

#### D07 CH1 映射的前后矛盾

- 文档：[p0-development-order.md:81](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/p0-development-order.md#L81)、[test-plan.md:93](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/docs/test-plan.md#L93) 写不要硬编码 CH1；后文又采用 SDK 推导映射并注明待测。
- 代码：`i2c_board.c` 写了 I2C3 映射。宏是否与 SDK 1.13.0 一致见 E02。
- 主类：文档过时或相互矛盾。影响：验收者不知道该信哪句。优先级：P1，`DOC`。建议：旧禁令改成“映射来自 SDK，板上未确认”。

#### D08 T1.3 macOS 实测与 README 口径不同

- 文档：DEV_PLAN L250 写 T1.3 macOS 已测；README 写仓库内无 USB 吞吐报告。
- 代码/记录：本 SHA 无对应原始日志。
- 主类：文档过时或相互矛盾。影响：吞吐数字无法复核。优先级：P2，`DOC`。建议：无日志则把 DEV_PLAN 该句降为口述记录。

#### D09 草稿规格与现行界面范围冲突

- 文档：PRD §4.2 与 xlsx SW-01 写 V1.0 界面只有 I2C，且无 macOS/Linux 上位机。README 与 UI 说明写跨平台 GUI，非 I2C 页是本地演示。
- 代码：GUI 同时有两类页面；非 I2C 页不发 USB，见 M13。
- 主类：文档过时或相互矛盾。影响：用 xlsx 会把演示页判成产品总线，或把跨平台 GUI 判成越界。优先级：P1，`DOC`。建议：对标表加“目标规格，不是实现清单”。

#### D10 AGENTS.md 未覆盖现行产品路径

- 文档：[AGENTS.md](https://github.com/kvvipa0028/USB_I2C_Hpm/blob/c41d6ab6273fe1d48eaf03cab384ca138009e2e9/AGENTS.md) 只有 usb_speed、i2c_proto、slave_proto 的构建与 T1.8/T1.9。
- 代码：产品固件是 `firmware/usb_i2c`，另有 HIL、siggen、控制协议。
- 主类：文档过时或相互矛盾。影响：新环境会构建错误固件。优先级：P1，`DOC`。建议：补一段入口，不把旧样例删掉。

### 3.6 证据不足（4）

#### E01 T1.8 通过记录只有叙述

- 要求：AGENTS.md 给出通过魔数 `c0ffee18`。
- 代码：`i2c_proto` 仍在。本 SHA 无原始 OpenOCD 日志。README 承认这一点。
- 主类：证据不足。影响：不能复述为本次通过。优先级：P2。建议：补日志或把叙述标成历史口述。

#### E02 CH1 SDK 宏未重核

- 要求：`i2c_board.c` 声称来自 SDK 1.13.0 pinmux。
- 代码：映射已写。本机无 HPM SDK 树，未对 `ccc5ec467a047f3ea85cd2fa4565e91db7135a38` 重算宏。CMake 只用 `HPM_SDK_BASE`。
- 主类：证据不足。影响：引脚可能与所用 SDK 不一致。优先级：P0，`HW`。建议：在锁定 SDK 的机器上 diff 宏，再上板。

#### E03 2026-11-07 主机日志不在本 SHA

- 要求：交接记录称 Rust 8/8、离线 51/51。
- 代码：用例数与 M11 一致。本 SHA 的 `outputs/` 无这些日志。本机 rustc 1.83.0 低于 README 的 1.95，未跑 cargo。
- 主类：证据不足。影响：主机测试次数不能由本次复述成新通过。优先级：P1，`ENV`。建议：在 1.95 上重跑并保存日志。

#### E04 GUI 与 Windows 历史次数未复跑

- 要求：`UI_MERGE_NOTES.md` 把 61 次标成历史 UI 记录。
- 代码：静态统计约 184 个 `#[test]`。未执行、未开窗口、未做 Windows。
- 主类：证据不足。影响：184 只是属性计数。61 不是当前失败。优先级：P1，`ENV`。建议：与 E03 同一次 cargo test 取出真实计数。

## 4. 最重要的差异与依赖顺序

1. **先改会带偏的文档，再写新功能。** D02、D04、D01 会让下一位实现者认为协议仍只有 PING/I2C、扫描/恢复仍是演示、D04 通道仍未确认。这三处与源码双向不符。
2. **单写者先于任何新的并发设计。** V01、V02、V03 是当前代码里的双写者。C1–C3 与单 trap 仍是未授权候选，不能当作已批准修复，也不能说成现码 UAF。
3. **SDK 宏先于双端口实板。** E02 未核之前，P02 的重叠和故障隔离测了也会被引脚疑问推翻。
4. **表服务语义先于 S3 验收签字。** P03 里 2000 µs 目前是广告和最大值，不是固件状态 9。
5. **仪器项集中做。** P01、P04、P05、P07 都卡在板、波形、Windows 或对端，软件命令已经存在。
6. **绑定、安装包、VID、Bootloader、真实 UDS 放在后面。** N01–N07 的现行文档已排除出本轮。真实刷写还依赖 D01 的决策口径先统一。

不构成当前偏差的排除项：C1 子队列公平、C2 过期定时反复唤醒及成本、C3 PINNED 租约复用 op、未授权单 trap、DMA 停止时限、USB 恢复时限、SDK 部署 SHA 未锁定。它们留在 PR #10 交接里，不进入 47 条分母。

## 5. 真实验证边界

### 5.1 本次执行的命令

环境：rustc 1.83.0（低于 README 要求的 1.95），默认 `cc` 为 clang 18.1.3 且无 compiler-rt ASan，gcc 13.3.0 带 libasan，cmake 3.28.3，Python 3.12.3，无 ninja。未安装工具链，未全量构建，未烧录。

| 时间 | 命令 | 结果 |
| --- | --- | --- |
| 2026-11-07 北京时间（审计窗口 19:39–19:49，UTC 11:39–11:49） | 仓库根目录 `python3 -m unittest discover -s tools/hil -p 'test_*.py' -v` | 18 tests，OK，0.004 s。不打开 USB，不编译 CLI |
| 2026-11-07 北京时间（同上窗口） | 默认 clang 配置 `firmware/tests` | 编译进行到链接后失败：找不到 `libclang_rt.asan_static-x86_64.a` 与 `libclang_rt.asan-x86_64.a`。这是本机 clang 缺 ASan 运行库 |
| 2026-11-07 北京时间（同上窗口） | `CC=gcc cmake -S firmware/tests -B /tmp/usb-i2c-native-gcc -G "Unix Makefiles" && cmake --build && ctest` | 4/4 通过：`common`、`protocol`、`port_state_machine`、`usb_slot`，约 0.07 s |
| 2026-11-07 北京时间（同上窗口） | 同样用 GCC 构建 `/tmp/siggen-native-gcc` | 5/5 通过：`command`、`peer`、`cdc_buffer`、`cdc_backend`、`recovery_gpio`，约 0.03 s |

构建目录只在 `/tmp`。未跑 `cargo test`、`cargo clippy`、GUI 窗口、交叉编译、OpenOCD、SDK 构建。历史 CI 与交接里的 279/51 次数不作为本次通过。

### 5.2 覆盖、抽样与未核

- 通读或近通读：PR #10 交接五份、README、PRD、DEV_PLAN、P0 顺序、控制协议、控制验收、测试计划、CLI 计划、USB 协议、S1、固件升级设计、UI 合并说明、UDS 提案状态与决策、刷写/ISO-TP 文首、CI、协议/端口/表/时序/恢复/板级/USB 槽。
- 抽样：GUI 以 `Job::` 与 `ControlRequest` 调用图为准，未逐页点击；siggen 核对表和 CDC 写点；xlsx 解析共享字符串，未跑公式引擎；T1.9 日志未逐字节重放。
- 未核：SDK 源码与 pinmux 宏、实板、Windows、热插拔、波形、WCET、1 MHz、10 分钟、Rust 测试实际计数。

### 5.3 锁定之后 main 是否变化

变化了。

| 项 | 值 |
| --- | --- |
| 开始 | 2026-11-07 19:39:39 北京时间（2026-11-07T11:39:39Z），`c41d6ab6273fe1d48eaf03cab384ca138009e2e9` |
| 结束 | 2026-11-07 19:49:38 北京时间（2026-11-07T11:49:38Z），`26723fe4497afee8df90ece7388d0b57097ff8f1` |
| 差异 | `.gitignore`；`outputs/20261106-doc-code-audit/文档与代码实现差距.md`；`outputs/p0-s3-s6-validation/验证报告.md`；同目录 Windows 两份报告；`outputs/windows-vm-validation/run-20261106-sogVXD/本地Windows验证报告.md` |
| 源码 | `git diff --stat c41d6ab..26723fe` 无 `.c`、`.rs`、`.py`、协议文档 |

新合入的 `文档与代码实现差距.md` 基线是 `169acb5526baaad5f794cc66a2de85e17635de9f`（2026-11-06）。它描述的单路 100 kHz、无扫描、无从机 USB 表，不符合 `c41d6ab`。不能把它当作本报告的当前差距清单。

`验证报告.md` 自称归档既有日志、没有重新执行测试，基线是 `43fd747` 加当时工作区，并写明 `hardware_pass=0`。原始日志被新的 `.gitignore` 排除，本报告不采纳其中的 279/279 作为新验证。

审计结束时本地 HEAD 仍停在锁定 SHA `c41d6ab`，工作树干净。本审计没有跟随到 `26723fe` 重做源码核对。事后把本文放入 `docs/` 时，分支基于当时的 `26723fe`，本文自身仍不进入 47 条矩阵，也不把该次归档说成对最新 `main` 的穷尽重审。
