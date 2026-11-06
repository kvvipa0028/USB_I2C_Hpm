# UDS 刷写传输层（ISO-TP / CAN、CAN FD）现成实现调研

| 项目 | 内容 |
| --- | --- |
| 日期 | 2026-11-05 |
| 范围 | ISO 15765-2（ISO-TP）在经典 CAN / CAN FD 上的现成实现；设备固件（C）和上位机（Rust，附 Python）两侧 |
| 不含 | DoIP、LIN、I2C 传输；UDS 应用层流程（见 [uds-bootloader-plugin-proposal.md](uds-bootloader-plugin-proposal.md)） |
| 性质 | 资料调研，未在本仓库集成或上板验证；不修改 PRD / DEV_PLAN |

## 1. 结论

1. **现成实现两侧都有，不需要从零写 ISO-TP。** HPM 官方没有：本机 `hpm_sdk` v1.13.0 中没有 ISO-TP / UDS 代码；`hpm_apps` 的 OTA 方案只支持 UART / USB / ENET 通道。所以只能集成第三方库。
2. **固件侧首选 `SimonCahill/isotp-c`**（MIT，C99，不用堆、不依赖 OS，支持经典 CAN 和 CAN FD 64 字节）。移植时实现 3 个钩子即可，CAN 收发用 HPM SDK 的 `hpm_can_drv`（HPM6360 的 CAN IP 是 `CAN_Type`，不是 MCAN）。如果以后固件里还要跑 UDS 客户端（例如脱机刷写），可用 `driftregion/iso14229`，它内嵌的就是这个 isotp-c。
3. **上位机侧首选 `automotive` crate**（MIT，Rust，自带 ISO-TP + UDSClient，支持 CAN FD）。它的 `CanAdapter` trait 是公开的，可以为本设备写一个适配器。代价是要引入 tokio，而且版本还是 0.x。**最终未采用（2026-11-05）：** ISO-TP 放在固件侧（见第 5 条），UDS 会话层也决定自己用 Rust 写（[端到端计划 2.1 节](uds-flash-implementation-plan.md)第 2 条）。
4. **不建议采用：** `ecu_diagnostics`（GPL-3.0-only，与本仓库 MIT 许可及商业分发冲突）；基于 Linux 内核 ISO-TP 的封装（只能在 Linux 上用）；Zephyr ISO-TP（需要 Zephyr RTOS，本仓库固件是裸 HPM SDK）。
5. 传输层放在哪一侧，需要按第 5 节的权衡决定。这个选择对应方案文档里尚未确认的 D04 / D05。**已决定（2026-11-05）：放在固件侧**，计划见 [uds-isotp-firmware-plan.md](uds-isotp-firmware-plan.md)。

## 2. 标准版本

| 标准 | 状态 | 来源 |
| --- | --- | --- |
| ISO 15765-2:2024（第 4 版，47 页） | 现行，2024-04 发布；ISO 已标记为“待修订” | [iso.org 84211](https://www.iso.org/standard/84211.html) |
| ISO 15765-2:2016（第 3 版） | 已作废，由 2024 版替代 | [iso.org 66574](https://www.iso.org/standard/66574.html) |
| ISO/CD 15765-2（第 5 版） | 委员会草案，将替代 2024 版 | [iso.org 94385](https://www.iso.org/standard/94385.html) |

2024 版摘要写明：本标准不规定其他标准应采用 CAN CC、CAN FD 还是两者都用。所以经典 CAN 和 CAN FD 哪个必须支持，要看目标 ECU 的规范来定。

## 3. 设备固件侧（C，HPM6360）

| 实现 | 许可 | 活跃度 | CAN FD | 与本项目匹配度 |
| --- | --- | --- | --- | --- |
| [SimonCahill/isotp-c](https://github.com/SimonCahill/isotp-c) | MIT | 最近推送 2026-10-05，74 star，有 CI 和 libFuzzer 目标 | 支持，TX_DL 最大 64，支持 12 位及 32 位转义 FF 长度 | **高**：C99、无堆、无 OS，只需 `isotp_user_send_can()`、`isotp_user_get_us()`、`isotp_user_debug()` 三个钩子；全双工、轮询驱动；可选分块接收（超出缓冲区的报文）；每条链路可带用户参数，用来区分多路 CAN |
| [driftregion/iso14229](https://github.com/driftregion/iso14229) | MIT | 最近推送 2026-09-29，373 star，文档版本 0.11.0 | 取决于所选 TP（内嵌上面的 isotp-c） | **中**：UDS 服务端和客户端都有，两文件发布，静态内存；客户端已支持 0x10/0x11/0x22/0x27/0x28/0x2E/0x31/0x34/0x35/0x36/0x37/0x38。API 仍是 0.y.z、**不稳定**。适合固件里做 UDS（例如脱机刷写），单纯做传输层用不到它 |
| [lishen2/isotp-c](https://github.com/lishen2/isotp-c) | MIT | 最近推送 2024-04-11，291 star | README 未提及 CAN FD（未查源码） | 低：基本停更，被上一行的实现取代 |
| [Zephyr ISO-TP](https://docs.zephyrproject.org/latest/services/connectivity/canbus/isotp.html) | Apache-2.0 | 随 Zephyr 主线维护 | 支持（文档：经典 CAN 8 字节、CAN FD 64 字节） | 低：依赖 Zephyr 内核与 CAN 驱动模型，本仓库固件是裸 HPM SDK |
| [hartkopp/can-isotp](https://github.com/hartkopp/can-isotp) | 内核模块（GitHub 未识别许可） | 最近推送 2023-10 | 支持（ISO 15765-2:2016） | 不适用：Linux 内核模块，MCU 上不能用 |

HPM 侧现状（本机核实）：

- `~/hpm/hpm_sdk`（v1.13.0）中检索 `isotp|iso_tp|iso-tp|15765|uds_|14229`，只命中 tflm 测试数据等无关文件。
- HPM6360 有 `HPM_CAN0`、`HPM_CAN1` 两路（`CAN_Type`，`CAN_SOC_MAX_COUNT = 2`），用的驱动是 `drivers/inc/hpm_can_drv.h`，支持 CAN FD（`enable_canfd`、BRS、TDC）。`samples/drivers/can/` 有回环、双板 Echo、CAN FD 发送、滤波、时间戳等示例，可以作为 isotp-c 底层驱动的起点。HPM6300EVK 只引出 CAN1（PZ04 / PZ05）。SDK 里的 `hpm_mcan_drv.h` 是给其他系列用的，HPM6360 用不上。
- [hpm_apps OTA](https://github.com/hpmicro/hpm_apps/tree/main/apps/ota)：README 写明只支持 UART / USB / ENET 三种有线通道，不含 CAN 和 UDS。

## 4. 上位机侧（Rust，附 Python）

本仓库上位机为同步 Rust（`nusb` + egui），没有 tokio。要在 macOS / Windows 上运行，传输层必须是用户态实现，不能依赖 Linux 内核 ISO-TP。

| 实现 | 许可 | 版本 / 活跃度 | 评价 |
| --- | --- | --- | --- |
| [automotive](https://github.com/I-CAN-hack/automotive)（[docs.rs](https://docs.rs/automotive/latest/automotive/)） | MIT | 0.5.0 于 2026-10-28 发布（距今约一周），共 11 个版本，103 star | **首选**。用户态 ISO-TP，支持经典 CAN / CAN FD，负责分段、重组和流控；配置项包括收发 ID、bus、填充、扩展寻址、TX_DL、BS、接收 STmin、覆盖对端 STmin、BRS。超时只有一个统一值（N_As/N_Ar/N_Bs/N_Cr 共用，默认 1 s）。上层自带 `UDSClient`（可走 ISO-TP 或 DoIP）。公开 `CanAdapter` trait（`send(&mut VecDeque<Frame>)`、`recv() -> Vec<Frame>`，另有 `buffer_size`、`cancel_pending_tx` 等可选方法），可以为本设备实现。限制：异步 API、依赖 tokio；0.x 版本 API 可能变动 |
| [ecu_diagnostics](https://github.com/rnd-ash/ecu_diagnostics) | **GPL-3.0-only** | 0.107.5，236 star | 许可不合适，未深入评估（README 获取超时） |
| docan / iso15765-2 / iso14229-1（[jesses2025smith](https://github.com/jesses2025smith/iso-std-rs)） | MIT OR LGPL-3.0（crates.io 声明；GitHub 未识别许可） | 0.1.0，3–7 star | 未深入评估；使用量小 |
| can-hal-isotp、can-iso-tp、ace-can、autors-isotp | MIT/Apache-2.0 | 都是 2026 年新建，下载量几百、star 很少 | 只作参考，暂不作为依赖 |
| [uds_stack](https://github.com/luminartech/uds_stack)（uds_session 等） | MIT OR Apache-2.0 | 0.7.0，2026-10-19 首发 | UDS 会话层（no_std、sans-io）+ DoIP 映射；未看到 CAN 传输 crate |
| socketcan-isotp、tokio-socketcan-isotp | MIT 等 | 下载量大 | 封装 Linux 内核 ISO-TP，只能在 Linux 上用 |
| [python-can-isotp](https://github.com/pylessard/python-can-isotp) + [udsoncan](https://github.com/pylessard/python-udsoncan) | MIT | 315 / 740 star，2026 年仍在更新 | 最成熟的用户态实现之一：v2 用内部线程处理时序，可选忙等提高精度（`wait_func`），Windows 下用 `perf_counter`。适合方案文档里“后台进程”的路线；需要为本设备写 python-can 接口，并随产品分发 Python 运行时 |

`automotive` 的 `IsoTPConfig` 文档有两点与本设备的固件设计直接相关（原文见 [config.rs](https://github.com/I-CAN-hack/automotive/blob/main/src/transports/isotp/config.rs) 和 README）：

- `send().await` 会等待发送确认。接收流里包含发送回显（`frame.loopback`）；硬件不提供确认的后端由软件模拟。所以本设备固件最好能回报“某帧已真正发上总线”。
- `batch_send`：只有发送 STmin 为 0 时才批量下发连续帧；否则每帧确认后才发下一帧。也就是说，ECU 要求 STmin > 0 时，每个 CF 都要经过一次 USB 往返。**这是由文档推出的结论，未读源码（获取超时），也没有实测。**

## 5. 传输层放在哪一侧：权衡

| | A：上位机做 ISO-TP | B：固件做 ISO-TP |
| --- | --- | --- |
| 固件职责 | 收发原始 CAN / CAN FD 帧，回报发送确认和时间戳 | 在 `hpm_can_drv` 上跑 isotp-c；USB 上传完整 UDS PDU，并下发链路配置 |
| 现成组件 | `automotive`（实现 `CanAdapter`） | `SimonCahill/isotp-c` |
| 时序 | STmin、BS、FC 等待都要经过 USB 往返；STmin > 0 时逐帧确认，吞吐受 USB 往返时间限制 | 在 MCU 内完成，STmin 精度取决于 MCU 定时器；与 J2534 设备承担 ISO-TP 的做法一致（`automotive` README：`J2534NativeIsoTP` 把分帧和时序交给 PassThru 设备） |
| 开发量 | 固件小；上位机需引入 tokio | 固件需要新的 USB 命令（PDU、配置、状态、错误），还要按 FF 长度规划缓冲区 |
| 扩展 | 同一个上位机库还带 DoIP 和 UDSClient | 以后可以做脱机刷写（固件里再加 iso14229 客户端） |

原建议是先测 HPM6360 USB 的往返时间，再用总线采集检查连续帧间隔是否满足 STmin，然后再定 A 还是 B。如果目标 ECU 的 STmin 和 BS 比较宽松，A 的落地成本更低；如果实测不满足时序，或产品需要脱机刷写，就选 B。不论选哪种，固件都需要先打通 CAN 的原始收发，目前固件里还没有 CAN 命令。

**已决定（2026-11-05）：选 B**，计划见 [uds-isotp-firmware-plan.md](uds-isotp-firmware-plan.md)。

## 6. 本次未覆盖或未核实

- ISO 15765-2 的时序参数：已按用户提供的 ISO 15765-2:2011（第 2 版，只涵盖经典 CAN）核对（2026-11-05）。N_As/N_Ar/N_Bs/N_Cr 超时为 1000 ms，是下限，实际不晚于 +50%（表 16、8.7.1）。逐条落实见 [固件计划 4.4 节](uds-isotp-firmware-plan.md)。目标 ECU 的 `0x7E3` / `0x7EB` 落在 OBD 法规 ID 范围内，按 ISO 14229-3:2012 第 11.1 节，网络层时序可能要按 ISO 15765-4 取值；15765-4 原文未拿到，具体数值待定（同见 4.4 节）。2016 / 2024 版正文仍未看到。
- 没有核实 `automotive` ISO-TP 源码的实际行为，也没有读 `ecu_diagnostics` 的 README（两次请求都超时）。
- 各库都没有在 HPM6360 或本仓库 host 上编译，也没有做与真实 ECU 的互通测试。
- 商业实现（Vector、ETAS 等 AUTOSAR CanTp）和 SAE J2534 规范原文都没有调研。
