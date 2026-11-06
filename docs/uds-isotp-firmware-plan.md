# 固件侧 ISO-TP（CAN）实施计划

| 项目 | 内容 |
| --- | --- |
| 日期 | 2026-11-05 |
| 前置文档 | [传输层调研](uds-transport-layer-survey.md)、[UDS Bootloader 方案](uds-bootloader-plugin-proposal.md) |
| 上位计划 | [端到端实施计划](uds-flash-implementation-plan.md)：本文是其中的阶段 A |
| 状态 | 计划，未开始实现；不修改 PRD / DEV_PLAN / 功能清单 |

## 1. 已确认的决定

| 事项 | 决定 |
| --- | --- |
| ISO-TP 放在哪一侧 | 设备固件（HPM6360）；上位机只收发完整 UDS PDU |
| 固件形态 | 扩展现有 `firmware/usb_i2c`，协议升到 v2；PING / I2C 的格式和行为不变 |
| CAN FD | 代码同时支持经典 CAN 和 CAN FD；首期只用经典 CAN 500 kbit/s 做互通验收，FD 标为“未验证” |
| 原始帧记录 | 首期就做：固件保存带时间戳的收发帧，上位机读取 |
| 测试对端 | Vector VN1630A + 真实 ECU（含 Bootloader） |
| 目标 ECU 已知参数 | 请求 ID `0x7E3`，响应 ID `0x7EB`，填充 `0xAA`。从 ID 数值推断是 11 位标准帧、正常寻址（不带扩展地址字节），**待 ECU 资料确认**。其余信息后续提供 |

## 2. 范围

纳入：

- 固件：CAN1 驱动胶层、集成 `SimonCahill/isotp-c`、USB 协议 v2 命令、原始帧环形缓冲。
- 上位机：`host/crates/usb-i2c` 中的协议编解码、会话 API、CLI 子命令和单元测试。
- 台架验证：片内回环、与分析仪互通、与真实 ECU 做只读互通。
- `tools/isotp_peer/`：在 Windows 上通过 VN1630A 模拟 ECU 的 ISO-TP 对端和故障注入脚本（python-can + python-can-isotp）。

不纳入（属于[端到端实施计划](uds-flash-implementation-plan.md)的阶段 B–F，或后续工作）：

- UDS 会话层：P2/P2*、NRC 0x78、TesterPresent 调度。
- 刷写流程执行器和 GUI 接入（CAN 页、UDS 页的真实后端）。
- 固件周期报文、多路 ISO-TP 链路、DoIP / LIN。
- 任何擦除、写入或复位 ECU 的操作。

## 3. 现状与约束（已核实）

| 依据 | 事实 | 对计划的影响 |
| --- | --- | --- |
| `firmware/usb_i2c/src/usb_i2c.c` | 一次 OUT 对应一次 IN；主循环轮询 `usb_i2c_task()`；回复发出后才重新启用 OUT；收发缓冲各 512 B；I2C 调用是阻塞的 | 需要改为“延迟回复”，并加大缓冲 |
| `host/crates/usb-i2c/src/bridge.rs` | 每个端点同时只有一个传输；超时固定 1 s；任何 USB 超时都会使会话失效；`validate_ping` 只接受协议 1；同步时 IN 缓冲为 `HDR + 256` | 超时要按命令设定；需要接受协议 2；同步缓冲要放大 |
| `hpm_soc_ip.h` / `hpm_soc_feature.h`（HPM6360） | `HPM_CAN0`、`HPM_CAN1`，`CAN_SOC_MAX_COUNT = 2`，IP 是 `CAN_Type` | 驱动用 `hpm_can_drv.h`，不是 MCAN |
| `hpm_can_drv.h` / `.c` | 支持 CAN FD（`enable_canfd`、BRS、TDC）；STB 非阻塞发送在队列满时返回 `status_can_tx_fifo_full`；有非阻塞接收、验收滤波、时间戳 | 发送钩子把“队列满”映射为 `ISOTP_RET_NOSPACE` |
| `samples/drivers/can` | 有所有 CAN 实例的回环测试、中断模式回环、双板 Echo、CAN FD 发送、滤波、错误示例 | 可作为驱动胶层的起点 |
| `boards/hpm6300evk` | 只有 1 个 CAN FD 接口：`BOARD_APP_CAN_BASE = HPM_CAN1`，引脚 PZ04 / PZ05 | 只有一路 CAN；终端电阻情况未核实 |
| `boards/hpm6750evkmini` | `board.h` 中没有 CAN 定义 | EVKMini 不能当 CAN 对端 |
| `flash_xip.ld`（HPM6360） | ILM 128K、DLM 128K、AXI_SRAM 512K（含非缓存区） | 几十 KB 的缓冲没有压力，实际余量以链接 map 为准 |
| `firmware/usb_speed/src/usb_speed.c` | 单次 `usbd_ep_start_write/read` 用 16 KiB / 8 KiB 缓冲 | 仓库已经用过多包批量传输（`usb_speed` 尚无上板报告） |
| [VN1600 手册](https://cdn.vector.com/cms/content/products/VN16xx/docs/VN1600_Interface_Family_Manual_EN.pdf)、[VN1600 产品页](https://www.vector.com/en/product/vn1600/) | VN1630A 最多 4 通道：2 路内置 CAN 高速收发器（TJA1051 / 1051cap）+ 2 个 piggyback 插槽，两个双通道 D-SUB9；CAN FD 最高 8 Mbit/s | 经典 CAN 和 CAN FD 都能用它验收；接到哪个通道、D-SUB 引脚和 DIP 开关要按手册确认 |
| [XL Driver Library](https://www.vector.com/en/business-unit/testing-validation-diagnostics/products/network-interfaces/xl-driver-library/)、[python-can vector 后端](https://github.com/hardbyte/python-can/blob/develop/can/interfaces/vector/canlib.py) | XL Driver Library 免费，提供 `vxlapi.dll` / `vxlapi64.dll`，允许多个应用同时访问（CANoe / CANalyzer 可以和自写程序并行）；python-can 的 vector 后端在非 Windows 系统上直接报错 | **对端脚本只能在 Windows 上运行**，macOS 不行 |
| isotp-c（提交 `1fc19e2`，2026-09-12） | 有 `isotp_send_with_id()`、`isotp_receive()`、分块接收；README 说明 `isotp_send()` 返回前已复制负载 | 功能寻址可以复用物理链路；接收 OUT 缓冲后可以立刻重新启用 |

## 4. 固件设计

### 4.1 模块

| 模块（建议路径） | 职责 |
| --- | --- |
| `firmware/third_party/isotp-c/` | 固定在某个提交，保留 LICENSE；编译选项：`ISO_TP_MAX_CAN_FRAME_SIZE=64`、启用 `ISO_TP_USER_SEND_CAN_ARG` 和 `ISO_TP_USER_SEND_CAN_FLAGS`、关闭库内填充（填充由胶层按运行时配置完成） |
| `usb_i2c/src/can_port.c` | CAN1 开关和波特率；RX 中断把帧写入软件环（建议 256 帧，溢出计数）；STB 非阻塞发送；错误计数和总线关闭（bus-off）状态 |
| `usb_i2c/src/isotp_glue.c` | 一条物理链路；实现 isotp-c 的三个钩子（发送、µs 时钟用 MCHTMR、调试输出）；发送钩子根据链路上下文决定 11/29 位 ID、FD/BRS，经典帧按配置填充到 8 字节；接收完成后把 PDU 放入 RX 队列（深度 2，满了丢弃并计数） |
| `usb_i2c/src/trace.c` | 变长记录环（64 KiB）：`ts_us u32, id u32(bit31=29位, bit30=FD, bit29=BRS), dir u8, len u8, data[len]`，按 4 字节对齐；满了丢弃新记录并计数 |
| `usb_i2c/src/usb_i2c.c` | 新命令解析、延迟回复状态机；USB 复位或断开时中止传输并关闭 CAN |

主循环每轮依次做：处理 USB 请求 → 从 RX 软件环取帧（全部写入 trace；与 `rx_id` 匹配的帧交给 `isotp_on_can_message`）→ `isotp_poll` → 把完成的 PDU 放入队列 → 检查延迟命令是否已经完成或超时。

### 4.2 设计约束

- **接收不丢帧，时序可能被拖慢。** I2C 命令会阻塞主循环（100 kHz 下 256 字节约二十几 ms）。这期间 RX 中断继续收帧，但 FC 的发出会被推迟。ECU 一侧 N_Bs 默认是 1 s，所以能接受。进行中的 ISO-TP 命令不会和 I2C 同时执行（见 4.3 的中止规则）。
- **STmin 精度取决于主循环轮询频率。** F1 和 F3 实测；如果不够，再把 `isotp_poll` 改到定时器中断里驱动。
- **TX 记录的时间戳不是上总线时间。** 它记的是帧交给 STB 的时刻，用于排查；判断时序以分析仪采集为准。
- **验收滤波首期全部接收**，这样 trace 才完整。在繁忙总线上要看溢出计数，必要时再改。

### 4.3 USB 协议 v2

请求和回复的 8 字节头保持不变。新命令里，头部的 `addr` 字节作为 `p0`（标志位），`wlen` 表示负载长度，`rlen` 表示上位机能接受的最大回复数据长度。固件的回复从不超过 `rlen`（沿用现有规则）。两个方向的缓冲都从 512 B 改为 4608 B（9 × 512）。

| cmd | 名称 | 请求负载 | 回复数据 | 回复时机 |
| --- | --- | --- | --- | --- |
| 0x01 | PING | — | 版本 `2` + `"usb_i2c 0.2"` | 立即 |
| 0x10 | I2C | 不变 | 不变 | 立即 |
| 0x20 | CAN_OPEN | `nominal_bps u32, data_bps u32(0=经典), flags u8(bit0 FD)` | — | 立即；清空 RX 环、RX 队列、trace |
| 0x21 | CAN_CLOSE | — | — | 立即；中止 ISO-TP |
| 0x22 | CAN_STATUS | — | `state u8(0关/1主动/2被动/3 bus-off), tec u8, rec u8, rsvd u8, rx_ring_drop u32, rxq_drop u32, trace_drop u32, last_err u32` | 立即 |
| 0x30 | ISOTP_CONFIG | `tx_id u32, rx_id u32, func_id u32(0xFFFFFFFF=无), flags u8(bit0 29位, bit1 FD, bit2 BRS, bit3 填充), pad u8, tx_dl u8, bs u8, stmin u8(FC 原值), rsvd u8, n_bs_ms u16, n_cr_ms u16` | — | 立即；重置链路并清空 RX 队列；有传输进行中时返回 BUSY；以下情况返回 BAD_REQ：`stmin` 为保留值（0x80–0xF0、0xFA–0xFF）、`n_bs_ms` / `n_cr_ms` 不在 1000–1500 之间（见 4.4），或 vendored 版本不支持该取值 |
| 0x31 | ISOTP_SEND | `p0 bit0=功能寻址`；`timeout_ms u32` + PDU（1–4095 B） | `ts_us u32`（最后一帧发送完成的时刻） | **延迟**：发送完成、失败或 `timeout_ms` 到期 |
| 0x32 | ISOTP_RECV | `timeout_ms u32` | `ts_us u32`（最后一帧收到的时刻）+ PDU | **延迟**：队列中有 PDU 就立即回复，否则等到超时 |
| 0x40 | TRACE_READ | — | `dropped u32` + 若干条完整记录（取出即删除） | 立即 |

在现有状态码 0–4 之上新增：`5 NOT_OPEN`（CAN 未打开或未配置 ISO-TP）、`6 BUSY`、`7 CAN_ERR`、`8 BUS_OFF`、`9 ISOTP_ERR`、`10 ABORTED`。另外，`2 TIMEOUT` 用于两种情况：RECV 到期没有数据、SEND 超过 `timeout_ms`。

`ISOTP_ERR` 的 `hpm_stat` 字段放本协议自己定义的 N_Result 码，名称取自 ISO 15765-2:2011 第 7.3.7 节，与具体库无关，胶层负责映射：`1 N_TIMEOUT_A`、`2 N_TIMEOUT_Bs`、`3 N_TIMEOUT_Cr`、`4 N_WRONG_SN`、`5 N_INVALID_FS`、`6 N_UNEXP_PDU`、`7 N_WFT_OVRN`（本机不发 WAIT，正常不会出现）、`8 N_BUFFER_OVFLW`、`9 N_ERROR`。接收失败（`N_TIMEOUT_Cr` / `N_WRONG_SN` / `N_UNEXP_PDU`）作为一条错误记录放进 RX 队列，下一次 RECV 返回 `ISOTP_ERR` 和对应的码，而不是让上位机白等到超时。

需要明确的规则：

- **延迟回复期间 OUT 保持启用。** 等待期间如果收到任何新请求，就先中止进行中的 SEND/RECV，并按原 seq 回复 `ABORTED`，然后再处理新请求。两个 IN 回复按顺序发出，固件需要记录 IN 是否忙。这样上位机进程崩溃后重连，PING 也能立即打断旧命令；现有 `synchronize` 会丢弃第一个旧回复，正好覆盖这种情况。
- **SEND 被中止或超时**时，如果最后一帧已经发送完成，就报告成功；否则报告 `ABORTED` / `TIMEOUT`，ECU 会因为 N_Cr 超时丢弃这个不完整的 PDU。只有在 USB 层失败时，结果才算“未知”。
- **功能寻址**用 `isotp_send_with_id(func_id)`，只允许单帧；超长时返回 BAD_REQ。物理链路正在多帧发送时返回 BUSY。
- **PDU 上限是 4095 B**，覆盖经典 CAN 12 位 FF_DL 的全部范围。上位机要把 TransferData 的块长限制在 min(ECU 的 maxNumberOfBlockLength, 4095) 以内。CAN FD 大于 4095 的长度等 FD 验收时再定。
- **USB 复位或断开时**：中止 ISO-TP，关闭 CAN，清空队列。设备不会自己重新打开总线。

### 4.4 ISO 15765-2:2011 条款落实

依据是用户提供的 ISO 15765-2:2011（第 2 版）。这一版只涵盖经典 CAN；CAN FD 和超过 4095 字节的 FF_DL 是 2016 版以后才加入的。现行版本是 2024 版（见调研文档）。首期经典 CAN 的行为按 2011 版核对；2011 版与 2024 版在经典 CAN 上有什么差异，未核对。

| 条款 | 要求 | 本设计的落实 | 验证 |
| --- | --- | --- | --- |
| 8.5.2.2 | SF_DL 为 0，或正常寻址下大于 7 的 SF，直接忽略 | F0 核实 isotp-c 的行为 | F3 注入 |
| 8.5.3.2 / 8.5.3.3 | FF_DL 最大 4095；正常寻址下 FF_DL < 8 时忽略，且不回 FC；FF_DL 超过接收缓冲时回 FC.OVFLW | 接收缓冲 4095 B，经典 CAN 下不会溢出 | F3 注入 FF_DL = 7、4095 |
| 8.5.4 | SN 从 1 开始，0xF 之后回到 0；SN 错误时中止接收，报 N_WRONG_SN | isotp-c | F3 |
| 8.5.5.2 / 8.5.5.3 | 发送方必须支持 CTS、WAIT、OVFLW：WAIT 重启 N_Bs；OVFLW 时中止并报 N_BUFFER_OVFLW；FS 为保留值时中止并报 N_INVALID_FS | isotp-c | F3 |
| 8.5.5.5 | STmin：0x00–0x7F 表示毫秒，0xF1–0xF9 表示 100–900 µs，其余为保留值。**计时从上一个 CF 发送完成开始** | 如果 isotp-c 从“交给驱动”的时刻开始计时，会少算一帧的发送时间（500 kbit/s 下 8 字节帧约 0.25 ms），实际间隔就会小于 STmin。处理办法二选一，F0 时决定：STmin > 0 时让发送钩子等上一帧发完再返回（带 N_As 超时）；或修改 vendored 版本的计时起点 | F3：用 VN1630A 时间戳量“上一 CF 结束 → 下一 CF 开始”，按时间戳打在帧起始还是帧结束换算 |
| 8.5.5.6 | 收到保留的 STmin 值时，本次传输按 127 ms 发送 | F0 核实 isotp-c | F3 注入 0x80、0xFA |
| 8.3 | 客户端作为接收方时，同一报文的各个 FC 必须用相同的 BS/STmin；作为发送方时，必须按每个 FC 的值调整 | 传输进行中禁止修改配置；发送时每收到一个 FC 都重新读取 BS/STmin | F3：对端在同一报文内改变 BS/STmin |
| 8.6 / 8.7.4 | N_WFTmax 只约束接收方；发送方每收到一个 WAIT 就重启 N_Bs，没有次数上限 | 本机作为接收方不发 WAIT（N_WFTmax = 0）。F0 核实 isotp-c 作为发送方时是否设了 WAIT 次数上限（`ISO_TP_MAX_WFT_NUMBER`）；如果有，设为不限制或修改代码，并记录 | F3：对端连发 10 个 WAIT 再发 CTS，应成功；对端一直发 WAIT，应由 `timeout_ms` 结束 |
| 8.7.1 表 16 | N_As、N_Ar、N_Bs、N_Cr 的超时都是 1000 ms。超时值是下限，实际超时不得晚于 +50%（≤ 1500 ms）。性能要求：(N_Br + N_Ar) 和 (N_Cs + N_As) 都要小于对应超时的 0.9 倍 | `n_bs_ms`、`n_cr_ms` 只接受 1000–1500；I2C 阻塞约二十几 ms，远小于 900 ms | F3：对端不回 FC、或中途停发 CF，测到的超时落在 1000–1500 ms |
| 8.7.2（N_As） | 一帧在 1000 ms 内没有发出去（例如总线上没有节点回 ACK），中止并报 N_TIMEOUT_A | 胶层跟踪每帧是否发送完成；超时就撤销 STB 中的帧并上报 | F3：断开对端，制造无 ACK |
| 8.7.3 表 18 | 全双工下：接收进行中又收到同一地址的 SF/FF，以 N_UNEXP_PDU 结束当前接收，并把新帧当作新报文处理；空闲时收到的 CF、不在发送中时收到的 FC，都忽略 | F0 核实 isotp-c 的行为 | F3 注入 |
| 9.4.2.1 / 9.4.3 | 采用填充方案时，DLC 恒为 8，包括 SF、FC 和最后一个 CF；采用填充方案的接收方会忽略 DLC < 8 的帧 | ECU 用 `0xAA` 填充，所以本机发的每一帧都必须是 DLC 8（F3 通过条件之一）；本机收到 DLC 小于 N_PCI 所需长度的帧时忽略 | F3 |
| 附录 B | 11 位 ID 中 0x7F4–0x7F6、0x7FA–0x7FB 由 ISO 保留 | 配置这些 ID 时给出警告 | — |

### 4.5 上位机（`host/crates/usb-i2c`）

- `proto.rs`：为上表各命令写编码和解码，扩展 `Status`；单元测试覆盖字节布局和边界（PDU 0 / 4095 / 4096、功能寻址 8 字节、超时上下限、截断的回复）。
- `bridge.rs`：
  - `transact` 改为按命令传入超时；SEND/RECV 用 `timeout_ms + 1 s`。
  - `validate_ping` 接受 1 和 2 并记录版本；协议 1 的设备调用 CAN 接口时直接报错。
  - 同步时的 IN 缓冲改为 4608。
  - `fail()` 遇到 ISOTP_SEND 时注明“PDU 可能已送达 ECU”。
  - 新增接口：`can_open / can_close / can_status / isotp_config / isotp_send / isotp_recv / trace_read`。
  - 用现有 Transport 队列模型补测试：延迟回复、同步时收到 `ABORTED` 旧回复、大回复的排空。
- CLI：`can open|close|status`、`isotp config`、`isotp send`、`isotp recv`、`isotp req`（发送一次加接收一次，打印十六进制，不处理 0x78）、`trace`（导出为 CSV）。

## 5. 分阶段与验证点

| 阶段 | 工作 | 通过条件 |
| --- | --- | --- |
| F0 准备 | 已有：VN1630A，ECU 的 `0x7E3` / `0x7EB` / `0xAA`。还需要：一台装好 Vector 驱动、XL Driver Library 和 Vector Hardware Config 的 Windows 电脑（CANoe / CANalyzer 可选）；VN1630A 用哪个通道及 D-SUB 接线；台架两端各有一个 120 Ω 终端（EVK 和 VN1630A 是否内置终端都未核实）。ECU 待补：确认 11 位和正常寻址、功能寻址 ID、可安全读取的 DID 列表、P2/P2*、参考工具记录。vendor isotp-c 并核实运行时可配项（BS、STmin、超时、FD 填充值），再按 4.4 节逐条核实标准行为：STmin 计时起点、收到保留 STmin 时的处理、发送方是否有 WAIT 次数上限、非法 SF_DL/FF_DL、DLC 不足的帧、意外 PDU、N_As 超时；与标准不符的地方修改 vendored 代码或在胶层补上，并记录差异 | 进入 F3 前，Windows 对端环境和接线要到位；进入 F4 前，ECU 资料要齐全；如果 isotp-c 只能在编译期设置 BS/STmin/超时，首期固定为 BS=0、STmin=0、1000 ms，配置命令拒绝其他值，并写进本文 |
| F1 片内自测 `firmware/isotp_proto` | 沿用 i2c_proto 的做法，独立固件，结果放在 `g_result` 里通过 JTAG 读取；复用 `usb_i2c/src` 的 `can_port.c` 和 `isotp_glue.c`。CAN1 内部回环，在同一控制器上建两条链路（`0x7E3` ↔ `0x7EB`，填充 `0xAA`，与目标 ECU 一致）对发。矩阵：长度 {1,7,8,62,63,4095} × 两个方向 × BS {0,1,8} × STmin {0, 0xF5, 1 ms}（SN 回绕在 4095 B 时自然覆盖）；FD 的 TX_DL 64 顺带跑一遍。另做外部冒烟：向分析仪发 10 帧、从分析仪收 10 帧 | magic 正确，0 处不一致，0 个协议错误，并记录耗时。**假设待验证**：内部回环会把本机发出的帧送进 RB，SDK 回环示例就是这样做的。如果不成立，F1 改为与分析仪对测 |
| F2 协议 v2 集成 | 4.3–4.5 的固件和上位机改动 | `cargo test --workspace --all-targets --locked`、`cargo clippy ... -D warnings`、Windows 目标 `cargo check` 全部通过；固件 release 构建通过。上板：PING 返回版本 2；按 README 已有检查做 I2C 回归（PING、写、重复 START 读、缺席地址 NACK）；`isotp recv --timeout 30000` 期间杀掉 CLI，重新运行 `ping` 能在 2 s 内成功 |
| F3 与分析仪互通（经典 CAN 500k） | 对端：Windows 上运行 `tools/isotp_peer`（python-can 的 vector 后端 + python-can-isotp，通过 VN1630A 模拟 ECU：在 `0x7E3` 上接收、从 `0x7EB` 发出、填充 `0xAA`），STmin 依据 VN1630A 的硬件时间戳判断。建议设备的 CLI 也在同一台 Windows 上运行，这样一个脚本就能编排每个用例；前提是先补做 README 中“待上板”的 WinUSB 检查。如果 WinUSB 不通，就由 macOS 运行 CLI、Windows 运行对端，逐个用例手工配合。用例：长度 1/6/7/8/62/63/64/4095 双向，4096 在本地被拒；对端 FC 的 BS 取 0/1/8，STmin 取 0、0xF1、0xF5、1、10、127 ms，以及保留值 0x80、0xFA；同一报文内对端改变 BS/STmin；对端连发 10 个 FC.WAIT 后发 CTS、对端一直发 WAIT；对端不回 FC；FC 报溢出；FS 为保留值；对端中途停发 CF；序号错误；接收中途对端插入新的 SF/FF；空闲时收到孤立的 CF/FC；对端发 SF_DL = 0 或 FF_DL = 7 的帧、DLC 不足的帧；断开对端（无 ACK，触发 N_As）；填充开/关和不同填充值；29 位 ID；功能寻址单帧；本机作为接收方时宣告的 BS/STmin；SEND 进行中发 PING 中止 | 本机发出的所有帧（包括 FC 和最后一个 CF）DLC 都是 8，填充字节都是 `0xAA`；数据逐字节一致；按分析仪时间戳，“上一 CF 发送完成 → 下一 CF 开始”都不小于 STmin，收到保留值时不小于 127 ms；N_Bs、N_Cr、N_As 超时落在 1000–1500 ms；每种故障都返回 4.4 节对应的 N_Result（意外 PDU、非法帧按标准忽略或重新开始），且下一个 PDU 能正常收发；同一次 4095 B 交换中，trace 与分析仪采集的帧数、顺序、ID、数据一致，dropped = 0；记录 4095 B 的发送耗时（不设目标值） |
| F4 真实 ECU 只读互通 | 按 ECU 资料配置（`tx 0x7E3`、`rx 0x7EB`、填充 `0xAA`，其余参数待补）；执行 `3E 00`、`10 01/03`，用 `22` 读单个 DID（长 DID 会触发多帧响应），用 `22` 一次读多个 DID 产生多帧请求；VN1630A 同时采集（用哪个工具在 F0 确定） | 响应与 ECU 规范或参考工具记录一致；没有 ISO-TP 错误；trace 与采集一致。ECU 不支持多 DID 时会回 NRC，但这仍能证明多帧发送已完成。**不做**任何擦除、写入或复位 |
| F5（首期之后） | VN1630A 的内置通道已支持 CAN FD，硬件不缺；按 F3 矩阵跑 CAN FD 500k/2M、TX_DL 64，并确定 FD 下的填充值 | 在此之前，FD 一律标为未验证 |

完成 F4 后，再进入方案文档的 P2：上位机 UDS 会话层和流程执行器。真实刷写所需的 ECU 资料见方案文档 D01。

## 6. 风险与待确认

| 风险 / 待确认 | 处理 |
| --- | --- |
| VN1630A 的驱动和 python-can vector 后端只支持 Windows | F3 的对端必须在 Windows 上运行；是否有可用的 Windows 电脑待确认 |
| 本设备的 WinUSB 在全新 Windows 上尚未验收（README 待上板） | F3 前先补做；不通时退回“两台电脑、逐个用例手工配合” |
| isotp-c 的运行时配置能力（BS、STmin、超时、FD 填充值）未逐项核实 | F0 vendor 时核实；不支持的取值由协议拒绝，不悄悄改值 |
| isotp-c 某些行为可能与 ISO 15765-2:2011 不一致（STmin 计时起点、发送方的 WAIT 上限等） | F0 按 4.4 节逐条核对；不一致的地方改代码或在胶层补上，并记录 |
| 主循环轮询下的 STmin 精度 | F1 / F3 实测；不满足时改为定时器驱动 |
| 全部接收的滤波设置在繁忙总线上可能溢出 | CAN_STATUS 暴露计数；出现溢出后再加硬件滤波 |
| EVK 和 VN1630A 是否自带终端电阻 | F0 查原理图和 VN1600 手册，或用万用表在 CANH–CANL 间实测（总线断电时应约 60 Ω） |
| ECU 的寻址方式、功能寻址 ID、DID、P2 尚缺 | 不影响 F0–F3；F4 前补齐 |
| PRD 把 CAN 列为 V2，需要新硬件 | 本计划只在 EVK 上做原型；产品硬件和产品字符串（仍叫 `usb_i2c`）留待产品决定，本文不改规划文档 |
