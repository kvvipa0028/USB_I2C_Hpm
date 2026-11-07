# PR10 厂商问题清单

清单尚未发送。本次记录不授权对外联系。各题需区分HPM6360与HPM6750，答复需给出版本、寄存器条件和时间保证。表内C、D、G、B、E、H标识对应证明主题，不表示主题已通过。V_Q、V_R、V_S、V_W、V_L分别指flush静止、reset静止、SETUP竞争、token回写、屏蔽下锁存保证，见[证明状态](proof-status.md)。

手册行号仅辅助定位，官方URL、版本、页码和hash见[来源索引](sources.md)。缺关键保证时保持未证，不把永久隔离当作可恢复合同已满足。

**总请求（HPM6360）**：请提供 HPM6360 用户手册中 USB 设备控制器、MCHTMR、I2C、PLIC/PLICSW 章节；或书面确认这些 IP 与 HPM6750 UM Rev2.6 所述完全相同（含版本差异）。没有它，下列每题在 6360 上都要单独回答。

## USB 设备控制器（6750 与 6360 分别回答）

| ID | 问题 | 依赖 |
|---|---|---|
| Q-U1 | 设备模式下对端点 b 写 ENDPTFLUSH 后，读到 ENDPTFLUSH[b]=0 且 ENDPTSTAT[b]=0，是否保证控制器之后**不再**读写该端点 QHD、任何 dTD 及其数据缓冲？若不保证，哪个可观察条件（寄存器/位/采样次序）能保证？从写 FLUSH 到该条件成立的最大时间 τ_q 是多少？（UM L88786–88788 只说在途包"continues until completion"） | DMA与租约、DMA与租约、SETUP与EP0（V_Q） |
| Q-U2 | 写 USBCMD.RST=1 并读回 0 之后，是否保证全部 USB DMA（描述符读取、数据读写、token 回写）已结束且不再写内存？RST 最大完成时间？仅写 RS=0 是否停止 DMA？ | DMA与租约（V_R） |
| Q-U3 | 请提供 UM L88589–88595（USBMODE.SLOM）引用的"Control Endpoint Operation Model"原文。具体：SUTW 被硬件清除的"hazard"精确定义；每次把 SETUP 写入 QHD setup 区是否都置 ENDPTSETUPSTAT 并清 SUTW；软件在新 SETUP 写入过程中清 ENDPTSETUPSTAT 时会怎样 | SETUP与EP0、SETUP与EP0（V_S） |
| Q-U4 | dTD token 的 Active=0 回写是否保证晚于该 dTD 最后一次数据写入（CPU 从 noncacheable SRAM 读取时可见）？ | DMA与租约、DMA与租约（V_W） |
| Q-U5 | EP0 IN/OUT 仍挂着上一控制传输的 dTD 时收到新 SETUP，硬件是否自动 flush/退役这些 dTD？之后对旧缓冲是否还会有 DMA？软件应采取的次序？ | SETUP与EP0 |
| Q-U6 | 从总线复位开始（SE0）到 USBSTS.URI 置位的最大延迟 δ_latch；是否有任何硬件状态或联锁可让软件在 URI 锁存前得知复位已开始（例如 PORTSC1.PR 的时序） | 复位与epoch、复位与epoch |
| Q-U7 | ENDPTPRIME 位清零的最大时间；复位处理期间是否必须检查 PORTSC1.PR 或遵循厂商规定的复位流程（UM 文本中未找到） | trap及循环有界性、trap及循环有界性 |
| Q-U8 | USBINTR 对应位为 0 时，USBSTS 的 URI/UI/PCI/SLI 是否仍锁存？ | 调度与唤醒、trap及循环有界性（V_L） |
| Q-U9 | 控制器是否在内部缓存 QHD overlay，使内存中的 QHD 与控制器状态可能不一致？DMA 是否只访问 total_bytes 范围内的缓冲页（qTD buffer[1..4] 预置指向后续 4 KB 页时）？ | DMA与租约、DMA与租约（V_F） |

## MCHTMR（UM §5.2.1，txt L13001–13009 只写了"≥ 发中断"和"中断有效时写 compare 清中断"）

| ID | 问题 | 依赖 |
|---|---|---|
| Q-T1 | 写 MTIMECMP 完成时计数 < 新值，之后计数到达 ≥ 新值：MTIP 是否置位并**保持**到下一次写 MTIMECMP（还是脉冲）？ | MMIO与时基（P_T1） |
| Q-T2 | 若写入值 ≤ 当前计数：中断是否（重新）置位？何时（同周期、下一个计时器时钟、不置位）？"写清除"与比较的先后？ | MMIO与时基、CE-T1 |
| Q-T3 | RV32 核上 64 位 MTIME 是否可单次原子读？若按两个 32 位半字访问，读低半是否锁存高半？ | MMIO与时基（P_T3） |
| Q-T4 | 按两个 32 位半字写 MTIMECMP：每次半写是否都清除有效中断？比较是否使用中间值？推荐的写序？ | MMIO与时基（P_T4） |
| Q-T5 | MRET 恢复 MIE 后若 MTIP 挂起，是否保证在被打断上下文执行下一条指令前进入 trap？最大延迟？ | MMIO与时基（P_T5） |
| Q-T6 | HPM6360 的 MCHTMR 语义是否与上面相同？计时器时钟来源？ | MMIO与时基 |

## I2C（产品 6360 与 siggen 6750）

| ID | 问题 | 依赖 |
|---|---|---|
| Q-I1 | INTEN 对应位清 0 时，STATUS 的 ADDRHIT/BYTERECV/CMPL/FIFO 状态是否仍锁存？ | trap及循环有界性、调度与唤醒（V_L） |
| Q-I2 | 从机模式下关闭自动 ACK、BYTERECV 之后，控制器是否拉住 SCL 直到软件给出 ACK/NACK？最长可拉多久？外部主机不支持时钟拉伸时行为如何？ | A_STRETCH、调度与唤醒 |
| Q-I3 | 6360 I2C FIFO 深度与事件合并规则（产品注释只有 [pslave] L208） | trap及循环有界性 |

## 总线、次序、中断控制器

| ID | 问题 | 依赖 |
|---|---|---|
| Q-B1 | 外设寄存器（USB、I2C、GPIO、PLIC、MCHTMR）访问的最大延迟；USB PHY 时钟门控或挂起时访问 USB 寄存器是否可能无限挂起总线？ | trap及循环有界性、MMIO与时基、P_T2 |
| Q-B2 | 对 noncacheable SRAM 的写，在 `fence w,o`（或 `fence iorw,iorw`）之后，是否保证先于随后对 ENDPTPRIME 的写被 USB DMA 主设备看到？noncacheable PMA 是否经写缓冲？ | DMA与租约 |
| Q-B3 | 是否存在限制 USB DMA 地址范围的 IOMMU/MPU？（预期没有；用于确认 C5 的能力边界） | DMA与租约 |
| Q-P1 | PLIC claim 返回 0（SDK trap.c L159–160 注释）的条件；每个真实中断至多产生几次空 claim？ | 空claim进入数上界 |
| Q-P2 | PLICSW pending 在被 claim/complete 期间再次置位，是保留还是丢失？ | 调度与唤醒 |
| Q-N1 | 6360/6750 的 NMI 来源与路由（SDK start.S L151–154） | 初始化与异常 |

## 不属于厂商、属于产品合同原文核对的问题（产品合同原文，不是放宽）

| ID | 问题 |
|---|---|
| K-1 | 产品合同是否已规定外部 I2C 主机的速率/突发上限（A_I2C）与时钟拉伸要求（A_STRETCH）？ |
| K-2 | 是否规定主机 USB 复位最短间隔（A_USBRST）与 SETUP 到达率（A_SETUP）？ |
| K-3 | 原合同的"复位后不发旧请求"是以 ISR 观察到复位为参照（ORIGINAL_REVIEW F1 L142–151），还是要求物理复位时刻起零窗口？ |

未规定的，相应证明义务保持**未证**，不作默认。
