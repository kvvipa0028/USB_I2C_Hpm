# PR10 条件所有权映射

本表重列原审查35行类别。F1为O1–O10，F2为O11–O22，F3为O23–O35。新增对象另列。所有权映射是纸面条件，不是实现已迁移，也不是设计通过。

R0表示对应IRQ尚未开启的初始化阶段。截止后M是主循环，T是真正非向量、非抢占单一硬件trap及其同步调用，E是同步异常，N是NMI。T.B和T.N只是同一T内部的边界服务和嵌套回调，不是两个硬件写者。

对象按整个数组和整个外设寄存器计，含SET、CLR、TOGGLE别名。没有token、IE、逐索引、volatile或关中断豁免。按PLIC源号计写者时，本候选不合规。DMA记为硬件效应，不等于DMA停止证明。

原判定表示源码基线的审查结果，“放行”不表示本候选已验收。V_Q是flush静止保证，V_R是控制器reset静止保证，V_W是token回写次序，V_L是中断屏蔽下锁存，P_T是计时器前提，W_E是epoch回绕前提，均保留相应未证范围。详见[证明状态](proof-status.md)、[来源](sources.md)和[厂商问题](vendor-questions.md)。C1–C3的最终限定优先于早期容量或公平性推导。

## 1 原 §3.1 产品 USB 槽（F1，10 行）

| # | 原对象（原判定） | 运行期唯一写者 | 跨根传递 | 本修订 Δ |
|---|---|---|---|---|
| O1 | `configured`（必改） | **T**：并入 `bus_word` 的 configured 位 | T store_release，M load_acquire | Δ：与 epoch 合成单字，只在 T.B 提交（回调撤销和配置次序） |
| O2 | `rx_ready`（必改） | **T**：ARM_RX 业务结果体 | 结果槽 SPSC | Δ：结果与缓冲归还分离 |
| O3 | `rx_armed`（必改） | **M**：ARM_RX 请求槽 | 请求槽 SPSC | — |
| O4 | `tx_done`（必改） | **T**：TX 业务结果体 | 结果槽 SPSC | — |
| O5 | `epoch`（必改） | **T**：`bus_word` 的 epoch 位段 | 同 O1 | M 只比较私有 `m_seen_epoch` |
| O6 | `tx_busy`（放行） | **M**（由未完成 TX 请求推导） | — | — |
| O7 | `tx_epoch`（放行） | **M**（TX 请求字段） | — | — |
| O8 | `rx_len`（放行） | **T**（ARM_RX 结果体字段） | 结果槽 | — |
| O9 | `task_epoch`、`active`（放行） | **M** | — | — |
| O10 | `rx_buf[]` / `tx_buf[]`（放行） | `rx_buf`：CPU 写者**无**（M 只在归还后读）；`tx_buf`：**M**（只在归还后写）。两者有 DMA 效应 | 归还只凭 `buf_return[b] == m_gen[b]`（T 写，M acquire） | Δ：结果到达 ≠ 缓冲可复用；QUAR 时永不归还 |

I2C CMD（START/STOP/abort）属于 O21 的 I2C 寄存器整体，写者 T；线性化仍是条件设计，见[设计状态](design-status.md)。

## 2 原 §3.2 产品从机表与端口（F2，12 行，每端口 k）

| # | 原对象（原判定） | 运行期唯一写者 | 跨根 / 容量 | Δ |
|---|---|---|---|---|
| O11 | `table.generation`（必改） | **T** | 副本只进结果体 | — |
| O12 | `table.bytes[inactive][*]` RESYNC（必改） | **T**（`table.bytes[2][256]` 整体） | — | O12–O16 合并为整体，取消逐下标放行 |
| O13 | `table.bytes[inactive][patch]`（放行） | **T**（同上整体） | patch 数据随请求体拷进 T 私有 op | 同上 |
| O14 | `table.bytes[inactive][非 patch]`（放行） | **T**（同上整体） | — | 同上 |
| O15 | `table.bytes[inactive]` FROZEN（放行） | **T**（同上整体）；快照拷进 T 写的结果槽体 | M 发布后读 | 同上 |
| O16 | `table.bytes[active]`（放行） | **T**（同上整体） | — | 同上 |
| O17 | `active`、`mode`、`begin`、`end`、`sync_next`（放行） | **T** | CAS 比较与翻转同一步；`tbl_busy` 互斥 | Δ：表操作互斥写明 |
| O18 | `slave_state`、`offset`（必改） | **T** | — | — |
| O19 | `tx_start`、`tx_next`、`rx_first`（放行） | **T** | — | — |
| O20 | `master_state`、`transfer`、`result`、`sent`、`received`、`deadline`（放行） | **T** | timing/deadline 由 M 预算后放进请求 | — |
| O21 | 该路 I2C 寄存器整体 INTEN/STATUS/FIFO/CTRL/CMD/ADDR/SETUP/TPM（必改） | **T**（含 `apply_config`、`i2c_reset`、INTEN 门控、CMD 线性化） | M 不读不写；`busy` 经结果体 | `apply_config` 不再写 PLIC（原 L67/L74） |
| O22 | `g_bridge_irq_lock_ticks_max`、`g_bridge_table_service_us_max`（放行） | **M** | JTAG 读 | 锁删除后改测请求往返 |

## 3 原 §3.3 siggen（F3，HPM6750，13 行）

| # | 原对象（原判定） | 运行期唯一写者 | 跨根 / 容量 | Δ |
|---|---|---|---|---|
| O23 | `peer->table[]`（必改） | **T** | FILL/WRITE 数据随请求；READ 数据进结果 | — |
| O24 | `peer->offset`（必改） | **T** | — | — |
| O25 | `slave_state`、`pending_ack`、`cancelled_acks`（及 `rx_first`、`write_index`、`rejected`）（必改） | **T** | 延迟 ACK 用静态期限槽 | — |
| O26 | `clock_generation`、`clock_armed`、`clock_result`、`clock_observed`、`held_mask`、`clock_restore_pending`、`hold_*`（必改） | **T** | hold 超时用静态期限槽 | 依赖 P_T1–P_T5（不再依赖电平 MTIP） |
| O27 | `owner`、`owner_generation`（必改） | **T** | — | — |
| O28 | CDC `configured`（必改） | **T**：siggen 的 `bus_word` configured 位 | M acquire | 只在 T.B 提交 |
| O29 | `ep_tx_busy`（必改） | 拆为：**M** 写 TX 请求；**T** 写 TX 业务结果 | SPSC | CDC 复位回调清 busy **不**表示缓冲可复用 |
| O30 | CDC `buffer` 的 `rx_head`、`rx_count`、`connected`、`session_lost`、`tx_busy`、`tx_length`、`discard_*`（必改） | 拆为不同整对象：**T**：`rx_head`、`rx_produced`、`connected`、`session_epoch`、`discard_*`、`rxoverflow`、`overflow_seq`；**M**：`rx_consumed`、`seen_session_epoch`、`overflow_seen`、TX 请求字段 | `rx_count = rx_produced − rx_consumed` 由读者推导 | 不是下标划分 |
| O31 | `buffer.rx_tail`（放行） | **M** | — | — |
| O32 | `buffer.rx[]`（放行） | **T**（整体，只在未满时写） | M 只读 | — |
| O33 | `cdc_dtr`（放行） | **T** | — | — |
| O34 | `write_buffer[]`（放行） | **M**（只在 `buf_return` 相等后写）；DMA 效应：IN 读 | `buf_return` | Δ：取消"epoch 变化即可复用" |
| O35 | `PEER_I2C` 寄存器与 GPIO 方向（必改） | I2C 寄存器整体：**T**；GPIOB DO/OE/IE/IF/PL/TP 整体（含别名）与 IOC PB10/PB11 PAD/FUNC：**T** | — | siggen GPIOB/IOC 只由 T 写；`i2c_init_*` 的 WCET 未证 |

此外 siggen 的 `read_buffer[]`（不在原表）：CPU 写者**无**，DMA 效应 OUT 写；T 只在 OK 结果（缓冲已归还）后读。siggen M 侧：解析器、行组装、回复格式化、`txreject` → **M**；`master_phase`、轮次结果、`mode` 等配置与 STATUS 快照 → **T**（快照进结果体）。

## 4 产品 GPIO 与恢复对象

| 对象 | 运行期唯一写者 | 说明 |
|---|---|---|
| 产品 GPIOC DO/OE（HPM_GPIO0 端口 C 整体，含别名；PC11–14） | **M** | 只在 QUIESCE 结果被消费之后、RESUME 发出之前驱动；T 从不写 |
| 产品 IOC PAD/FUNC PC11–14（每个 pad 寄存器整体） | **M** | 同上 |
| `recovering`、`recovery` | **M** | — |
| `port_quiesced[k]` | **T** | 单字 release |
| `config`、`timing`、`config_generation`、`fault` | **T** | CAS、`busy`、apply 与自增同一步 |

产品源码运行期写 GPIO 的只有 i2c_board.c；SDK board 层、CherryUSB class/glue、日志后端的运行期写点**未审**，这是本表的覆盖边界。

## 5 新增对象：邮箱、租约、调度、DCD、平台

| 对象 | 唯一写者 | 读者 / 硬件效应 | 规则与回绕 |
|---|---|---|---|
| `req[NREQ]` 槽体、`req_head` | **M** | T acquire | 32 位自由计数，`head − tail ≤ NREQ ≪ 2^31` |
| `req_tail`、`res[NRES]` 槽体、`res_head` | **T** | M acquire | 槽体写完整再 release |
| `res_tail` | **M** | T acquire | 读完才 release |
| `req_tail_t`、`res_head_t`、`res_reserved`、op 池 | **T** 私有 | — | 活跃 op ≤ NRES（见[设计状态](design-status.md)） |
| `req_head_m`、`res_tail_m`、`m_gen[b]`、`m_seen_epoch` | **M** 私有 | — | `m_gen[b]` 只在归还后 +1 |
| `bus_word`（epoch31 + configured） | **T**（只在 T.B） | M acquire | 回绕前提 W_E（见[设计状态](design-status.md)） |
| `usb_avail`（控制器状态 + 端点可用位） | **T** | M acquire | — |
| `buf_return[b]`（每个 DMA 缓冲） | **T** | M acquire | 只比较相等；租借严格串行，无 ABA |
| 租约表（每在用端点方向一项）、`quar`、`ctrl_gen` | **T** 私有 | — | 静态容量，QUAR 不需要新容量 |
| 端点状态机、`uhw`、`pend_prime`、`pend_open`、`deferred_status`、`cfg_event`、`cfg_e0`、`cfg_value`、`revoke_pending`、`nest`、`setup_latch`、`usb_pend`、`cpl_pend`、`ep0_superseded` | **T** 私有 | — | REINIT 清 `pend_*`/`deferred_status`/`cfg_event`，保留 `setup_latch` |
| 调度：`frame_end`、`spent`、`B_T`、`cnt`、`q`、`gated`、`rr`、`urr`、`used`、`used_u`、`slices_this_frame`、期限槽、`tbl_busy`、诊断计数 | **T** 私有 | — | `frame_roll` 重置 `spent`/`used`/`used_u`/`slices_this_frame`/`cnt` |
| MCHTMR MTIMECMP（整体，两个半字） | R0 → **T** | 硬件比较；写清中断（UM §5.2.1） | M 只读 MTIME；MTIME 运行期无写者 |
| PLIC enable/priority/threshold | **只在 R0** | 硬件 | 运行期无写者 |
| PLIC CLAIM/COMPLETE、PLICSW CLAIM/COMPLETE | **T**（SDK trap.c 分支内） | claim 清 pending 是硬件效应 | PLICSW pending 在 claim 期间的保留语义未证 |
| PLICSW PENDING | **M** | 硬件 | 只作门铃，T 不依赖 |
| USB 控制器寄存器整体（USBCMD/USBSTS/USBINTR/ENDPT*/DEVICEADDR/ENDPTLISTADDR/USBMODE/PORTSC1/PHY） | R0 → **T** | 硬件置位/清除 | T.N 只读；W1C 清除只在 T.B |
| `_dcd_data`（QHD 数组、qTD 池）、`usb_device_handle` | R0 → **T** | DMA 效应 | PINNED/REVOKED/CLOSING/QUAR 期间 CPU 不写 |
| `g_hpm_udc`、`_setup_data`/`setup_buf`、`g_usbd_core`（含 `req_data`/`ep0_data_buf`） | R0 → **T** | `req_data` 有 DMA 效应 | `EP0_INDATA_NO_COPY` 宏未确认（C7） |
| `diag_E` / `diag_N` | **E** / **N** 各自 | 调试读 | 两个对象，不共用 |

## 6 CSR 平台背景（观测到的软件写点，如实列出；不是产品对象，不是豁免）

| CSR | 观测到的写点 | 说明 |
|---|---|---|
| `mstatus` | R0：截止点 `csrs MIE`；T：[trap.c L197](https://github.com/hpmicro/hpm_sdk/blob/ccc5ec467a047f3ea85cd2fa4565e91db7135a38/soc/HPM6300/HPM6360/toolchains/trap.c#L196-L198) trap 出口软件恢复；硬件：trap 进入/MRET 改 MIE/MPIE | **不能**写成"运行期 CPU 写者：无"。L162 提前开 MIE 被 `DISABLE_IRQ_PREEMPTIVE=1` 排除。E 不返回所以不执行 L197（条件：覆盖 `exception_handler`）；N 路径未知；M 运行期不写（ELF 未证）；`DTD_POOL_SHARED` 禁止（会在 T 内写 MIE） |
| `mepc` | T：L198；硬件：trap 进入 | 同上 |

这些 CSR 不纳入 35 行产品对象，也不构成 F1–F3 新必改；它们是平台调度背景，本表不伪造寄存器所有权证明。

## 7 本表不证明什么

不证明 DMA 终止、USB 可恢复、任何时延数值；不覆盖 SDK board/class/glue/日志后端的写点；不能用板测代替；"R0 → X"只表示截止之后唯一写者是 X。
