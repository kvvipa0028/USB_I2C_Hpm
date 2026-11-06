# S1 无硬件准备与上板验收手册

日期：2026-11-06。状态：试验准备；本轮无 EVK，不产生新增上板结论。

对应 [P0 计划 S1](p0-development-order.md)、[PRD](../PRD.md) 的 FR-SLV-02/03/05、FR-MST-02 和风险 R1/R2。本手册区分已经存在的 200 轮冒烟、需要补齐的连续测试和将来必须取得的波形证据；不把 host 模型或 target 编译当成实测。

## 1. 当前基线和可复用内容

| 项目 | 已存在 | 尚未证明 |
| --- | --- | --- |
| [slave_proto](../firmware/slave_proto/src/main.c) | I2C0、0x50、256 B 表、1 B 偏移、中断收发、BYTERECV 手动 ACK、读事务按实际发送量推进偏移 | 400k/1M 正确接线下零错误；最坏 IRQ 延迟 |
| [slave_test](../tools/slave_test/src/main.c) | 200 轮整表/随机/当前地址/回绕测试；每次读与 `g_exp` 比较；9-word `g_result` | 10 分钟连续负载；长轮号/计时扩展；USB/双端口压力 |
| [i2c_proto](../firmware/i2c_proto/src/main.c) | 100k 基本主机冒烟，SDK 对端 0x16，128 B 写后读及缺席地址检查 | 10k–1M 自定义频率计算与实际输出；该原型不是连续频率发生器 |
| [T1.9 记录](T1.9-slave-test.md) | 2026-11-01 正确直连的 100k 200 轮通过；400k 旧失败发生在接线/上拉异常期间 | 不能用旧失败判定 400k 不可行，也不能据 100k 通过关闭 R1 |

SDK 本地核对为 `1.13.0 @ ccc5ec46`。`drivers/inc/hpm_i2c_drv.h` 有 `i2c_mode_normal`、`i2c_mode_fast`、`i2c_mode_fast_plus`；`drivers/src/hpm_i2c_drv.c` 对应 100k/400k/1M。`soc/HPM6300/HPM6360/hpm_soc_feature.h` 定义 FIFO 4 B、单次传输最大 256 B。这些是实现依据，不是板上性能证据。

## 2. 无硬件时的工作与完成边界

1. 固定每组构建的仓库提交、SDK 提交、编译器版本、release 配置、宏值及 ELF 摘要；在独立构建目录生成三档主从镜像，不覆盖历史证据。
2. 将速率/轮数/对端上拉变成受校验的构建参数，保留当前默认值；检查配置确实进入编译命令和结果字段，不能只传一个被 CMake 忽略的 `-D`。
3. 保留 9-word 结果布局。当前 `first_info` 用高 16 位保存首错轮号，长轮数必须受限或另行设计完整轮号字段；不得让首错信息静默截断。
4. 为自定义频率建立纯计算测试输入/预期、整数溢出/无解拒绝测试和寄存器范围检查；最终算法需与实际时钟、SDK/芯片时序公式核对。这是后续实现入口，本手册不宣称算法已完成。
5. 准备下述台架矩阵、仪器设置和空白记录。无设备运行脚本时写 `PENDING_HARDWARE`/SKIP，不写 PASS。

本轮不接板、不运行 OpenOCD、不烧录、不改变 OTP。后文的硬件命令是到板后使用的模板。

## 3. 接线与仪器准入

| 信号 | HPM6300EVK（被测从机） | HPM6750EVKMini（外部主机） | 核对依据 |
| --- | --- | --- | --- |
| SCL | J28[13]，I2C0，PC13 | P1[13]，I2C0，PB11 | T1.9 接线记录；两板 SDK `pinmux.c` |
| SDA | J28[15]，I2C0，PC14 | P1[15]，I2C0，PB10 | 同上 |
| GND | GND | GND | 两板共地，确认实际连接器脚位 |

该映射针对已有两板记录。测试前核对 PCB revision、丝印和连通性；它不定义产品 CH1，也不证明不同板型可直接使用。

- 先在断电状态检查 SCL→SCL、SDA→SDA、共地和是否短路。接收方供电/IO 额定值确认后使用 3.3V 台架；不得向裸 EVK I2C 引脚直接施加 5V。
- SCL 和 SDA 各自独立上拉到同一已确认电平，记录实装阻值及内部上拉状态。SDK EVK `init_i2c0_pins()` 开内部上拉，EVKMini 默认不开；不能只记“加了一个电阻”。阻值按总线电容和波形选择，不预设某一档一定能通过 1MHz。
- 使用两路示波器同时看 SCL/SDA，记录探头/衰减/带宽/采样率、阈值和探头电容；逻辑分析仪用于 START/STOP/ACK、地址与数据解码。仅能解码不能替代模拟上升沿测量。
- 每档测量无拉伸数据段的 SCL 高低时间、周期、上升/下降沿及电平。若采集不到可信边沿或时基精度不足，频率结果标为未验证。
- 两个 FT2232 的 serial/JTAG ID 可能相同，必须现场按 USB location 绑定板名；`AGENTS.md` 的历史 location 只供识别参考，不能直接复用。一次只拔插一块板可协助确认，最终把 location 与板号写入记录。
- ISR 计时用芯片计时器或事先确认的空闲 GPIO 标记；GPIO 映射未确认前不随意占用引脚。记录测量开销，不能把 GPIO 脉宽直接当所有场景的 IRQ 响应延迟。

## 4. 构建与符号准备

在仓库根目录执行构建，所有性能目标显式使用 `-DCMAKE_BUILD_TYPE=release`。以下先给当前默认 100k 冒烟的独立目录示例；三档配置应使用本项目正式暴露并校验的构建选项，不通过修改 SDK 来切换档位。

```sh
source tools/env.sh
cmake -S firmware/slave_proto -B firmware/slave_proto/build-s1-100k -GNinja -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
cmake --build firmware/slave_proto/build-s1-100k
cmake -S tools/slave_test -B tools/slave_test/build-s1-100k -GNinja -DBOARD=hpm6750evkmini -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
cmake --build tools/slave_test/build-s1-100k
```

构建结束后从**该次 ELF**取符号，禁止复制旧地址：

```sh
"$GNURISCV_TOOLCHAIN_PATH/bin/riscv32-unknown-elf-nm" -S --defined-only firmware/slave_proto/build-s1-100k/output/demo.elf | rg ' g_(slave|table)$'
"$GNURISCV_TOOLCHAIN_PATH/bin/riscv32-unknown-elf-nm" -S --defined-only tools/slave_test/build-s1-100k/output/demo.elf | rg ' g_(result|exp)$'
shasum -a 256 firmware/slave_proto/build-s1-100k/output/demo.elf tools/slave_test/build-s1-100k/output/demo.elf
```

同时保存 map、CMakeCache、编译命令或 `ninja -t commands` 输出。编译失败和缺工具分别报告，不生成通过记录。已有 `build/` 中的旧 ELF 不能当作本次新配置产物。

## 5. 到板后的执行顺序

1. 核对第 3 节、匹配的主从速率及 `PEER_PULLUP`；确认被测地址 0x50 与频率测试用 0x16 不是同一套对端固件。
2. 先烧 HPM6300EVK 的 `slave_proto`，再烧 EVKMini 的 `slave_test`。用项目 `AGENTS.md` 中按 USB location 选板的流程，每次明确板型/ELF。
3. 对外部主机启动整个测试过程进行波形采集；先做 100k 冒烟，确认通过后再测 400k、1M。每次换速率都重新初始化两端，并保存独立记录。
4. 测试期间不 halt 任意一端，否则会改变实时行为。等待本次测试结束/到明确 deadline 后，先停止外部主机总线活动，再读两板结果。超时未结束按 FAIL/ERROR 排查，不反复 reset 后只保留成功那次。
5. 在 EVKMini 读取 `g_result` 9 words 和 `g_exp` 256 bytes；在 EVK 读取 `g_slave` 4 words 和 `g_table` 256 bytes。逐字节比较完整表，不只比较总和。

下面是**读数模板**，尖括号必须替换为本次确认的值；在 `$HPM_SDK_BASE/boards/openocd` 目录运行。每个动作使用独立 `-c`，尤其 `halt` 之后不要把多条命令塞进同一个 `-c`：

```sh
openocd -c "set HPM_SDK_BASE $HPM_SDK_BASE; set BOARD hpm6750evkmini; set PROBE ft2232;" -f hpm6750_all_in_one.cfg -c "adapter usb location <confirmed-peer-location>" -c init -c halt -c "mdw <g_result-from-peer-elf> 9" -c "mdb <g_exp-from-peer-elf> 256" -c resume -c shutdown
openocd -c "set HPM_SDK_BASE $HPM_SDK_BASE; set BOARD hpm6300evk; set PROBE ft2232;" -f hpm6300_all_in_one.cfg -c "adapter usb location <confirmed-evk-location>" -c init -c halt -c "mdw <g_slave-from-evk-elf> 4" -c "mdb <g_table-from-evk-elf> 256" -c resume -c shutdown
```

200 轮基线的 oracle：

- `g_result.magic=0xC0FFEE19`、`rounds=200`；`call_fail=mismatches=first_info=first_val=0`；mode 和 peer_pullup 与本次构建一致，elapsed_us 为实际读数。
- `g_table` 与 `g_exp` 256 B 完全一致；`g_slave.magic=0x51A7E001`。
- 对未改动的九调用测试序列，完成后计数应为 `addr_hits=12*rounds`、`rx_done=6*rounds`、`tx_done=6*rounds`。改测试序列后须重建预期，不能继续套这个倍数。
- `first_info` 按 `round << 16 | step << 8 | index` 解码；step 是测试序列编号，不能只看到 `first_val` 就猜失效原因。

## 6. R1：标准速率与连续整表测试

| 测试 ID | 内容 | 通过条件 | 当前证据 |
| --- | --- | --- | --- |
| S1-SLV-100K | 100k、200 轮现有九调用序列 | 上述结果/表/计数全部匹配 | 历史 T1.9 通过；当前构建仍待上板 |
| S1-SLV-400K | 正确接线/独立上拉后的 400k 同序列 | 零调用失败、零数据错误，波形确认真实档位 | PENDING_HARDWARE |
| S1-SLV-1M | 1M 同序列 | 同上，不能靠从机持续拉伸将总线降速后宣称 1MHz 持续能力 | PENDING_HARDWARE |
| S1-SLV-CONT | 每档连续至少 10 分钟，整表数据逐字节比较，穿插当前地址/短读/回绕 | 数据零错误、无停滞；完整记录时长/事务数/最大延迟/有效吞吐 | 测试运行时控制尚需补齐；PENDING_IMPLEMENTATION_AND_HARDWARE |
| S1-SLV-IRQ | 1M 最坏 IRQ 响应/处理与 FIFO 服务间隔 | 在所选实现的实际 FIFO/拉伸预算内，含测量开销说明 | PENDING_HARDWARE |

当前每轮已有：写 0..254、补写 255、随机读 256、当前地址读 4+4、0x80 随机读 10、当前地址读 6、从 0xFE 跨尾写/读 4。它覆盖整表、0xFF 回绕及预取不应改变实际读偏移；每轮数据模式会变化，不只是重复读零。

10 分钟是 [P0 计划](p0-development-order.md) 建议的早期连续门槛，不代替最终 1h/24h 长稳。单次 200 轮或反复 reset 后累加 10 分钟都不等价。受现有 16 位首错轮号约束，轮数配置上限应不超过 65535；1M 下该上限可能不足 10 分钟，必须先补齐保留兼容的长运行/完整结果方案再执行 S1-SLV-CONT，不能静默溢出或用软件延时凑“连续负载”。

4 B FIFO 在理想 1MHz、每字节含 ACK 的粗略时间是 `4*9/1MHz=36us`，只用于安排测量；实际中断触发阈值、入/出 FIFO、手动 ACK 和拉伸决定可用预算。不得直接把 36us 当成已验证的最大 ISR 延迟。连续性报告同时保留 SCL 低延长、事务间空隙和有效吞吐，避免“配置了 1M”替代“以目标速率连续运行”。

S1 仅验证独立原型；迁入产品后还需 S3/S4/S10 的 USB 表访问、双端口并发和最长主机等待压力。无 USB 的原型不能关闭这些项。

## 7. R2：自定义主机频率测量

目前 `i2c_proto` 仅通过 `board_init_i2c()` 使用 100k，SDK时序入口只接三档 enum。需要先实现项目自有的受检计算入口和可重复输出的测试模式；下面六点不是现有代码已经支持的命令。

| 测试 ID | 目标频率 | 测量窗口 | 状态 |
| --- | --- | --- | --- |
| S1-FREQ-010 | 10 kHz | 无从机拉伸的稳定数据时钟 | PENDING_IMPLEMENTATION_AND_HARDWARE |
| S1-FREQ-050 | 50 kHz | 同上 | PENDING_IMPLEMENTATION_AND_HARDWARE |
| S1-FREQ-100 | 100 kHz | 同上 | PENDING_HARDWARE |
| S1-FREQ-400 | 400 kHz | 同上 | PENDING_IMPLEMENTATION_AND_HARDWARE |
| S1-FREQ-800 | 800 kHz | 同上 | PENDING_IMPLEMENTATION_AND_HARDWARE |
| S1-FREQ-1000 | 1 MHz | 同上 | PENDING_IMPLEMENTATION_AND_HARDWARE |

测量步骤：

1. 记录外设实际输入时钟、选中的时序寄存器、计算频率与无解判据；取值不能来自固定假设 CPU 频率。
2. 对端使用已验证不会主动拉伸该测量窗口的设备/模式；普通 SDK 128 B echo 样例如产生拉伸，只能作基本互通，不能直接用整笔耗时测频率。
3. 在连续稳定 SCL 周期上记录周期均值和范围、tHIGH/tLOW、tr/tf；保留原始波形、阈值和仪器时基设置。排除 START/STOP 间隔和明确的拉伸窗口，同时另行记录它们。
4. `f_measured=1/T_mean`；`error_percent=abs(f_measured-f_requested)/f_requested*100`，六档均要求 ≤5%。不能使用含软件等待的总传输时间来声称 SCL 精度；测量不确定度跨越限值时不能判 PASS。
5. 高频还要核对适用模式的高低时间/上升沿限制；限制值和依据在正式测试记录中填写。只有频率均值达标但边沿/时序不合格仍不得通过。
6. 对合法范围边界、超界、极端输入时钟和寄存器无解情况跑计算测试；不允许截断、wrap 或悄悄换成最近标准档。最终前端/时钟源改变后重新实测。

## 8. 待补证据模板

每个用例单独复制下表到测试输出目录，保留失败和未测记录。空值不等于零，`PENDING_*` 不计通过。

| 字段 | 本次值 |
| --- | --- |
| Case ID / status | `待填` / `PENDING_HARDWARE` |
| 执行人、时间、仓库提交、dirty diff 标识 | 待填 |
| SDK 版本/提交/本地补丁、GCC版本、release配置 | 待填 |
| 主/从 ELF 路径与 SHA-256、map、编译命令 | 待填 |
| 两板型号/revision/板号、USB location、确认方式 | 待填 |
| SCL/SDA/GND 连线、供电、每线外部上拉/内部上拉 | 待填 |
| mode/目标频率、实际外设时钟、时序寄存器、ROUNDS | 待填 |
| 仪器型号/采样率/带宽/探头/阈值/时基精度 | 待填 |
| 运行开始/结束、持续时长、是否 reset/halt/人为插入延时 | 待填 |
| g_result/g_slave 原始 words，g_table/g_exp 完整 dump | 待填 |
| 调用失败、字节不一致、首错轮/步骤/值 | 待填 |
| 周期/频率误差、高低时间、上升/下降沿、拉伸/间隙 | 待填 |
| IRQ 最坏响应/处理时间、测量方法/开销 | 待填 |
| 波形/日志路径、独立 oracle 比对结果 | 待填 |
| 通过/失败理由、未覆盖项、后续动作 | 待填 |

S1 关闭 R1/R2 需要相关用例的真实板卡与波形证据全部达到原要求；纯计算、release 构建、符号存在只能分别标注准备工作的完成程度。
