# S4 CDC 参考从机与故障注入对端

`tools/siggen` 用于 HPM6750EVKMini 的 I2C0 对端验证，保留旧 `start/stop/status/help` 命令，并增加可编程从机、NACK、写数据阶段 clock stretch 和有期限的卡线。它不是产品固件，也不替代仪器波形证据。本轮未烧录、未连接对端。

## 板级边界与构建

SDK v1.13.0 `boards/hpm6750evkmini/pinmux.c:187–194` 和 `README_en.rst:106–116` 定义 PB11=P1[13] SCL、PB10=P1[15] SDA；GND 共地。仅允许构建 `hpm6750evkmini`，防止将固定的 fault GPIO 映射用于其它板。PB10/PB11 另有 ESP_HOSTED/handshake 用途；此固件按 I2C 测试专用理解，接线前核对板上实际占用。裸 EVK 引脚不代表 1.8–5V 产品前端，使用经确认的 3.3V 电平和外部上拉。

```sh
source tools/env.sh
cmake -S tools/siggen -B tools/siggen/build -G Ninja \
  -DBOARD=hpm6750evkmini -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
cmake --build tools/siggen/build
cmake -S tools/siggen/tests -B /tmp/siggen-native -G Ninja
cmake --build /tmp/siggen-native
ctest --test-dir /tmp/siggen-native --output-on-failure
```

产物为 `tools/siggen/build/output/demo.elf`。此文档不自动烧录；实际 probe USB location、板号与接线必须核实。初始化只使用 `board_init_i2c_clock`、pinmux 和 I2C driver init，不调用带启动清线的 `board_init_i2c()`，不会以初始化额外的 9 clocks 冒充故障恢复。

## CDC 命令合同

以 CR、LF 或 CRLF 结束，每条命令一条 `OK...` 或 `ERR...` 回复。命令区分大小写；数字严格接受十进制或 `0x` 前缀，不接受正负号、溢出、额外 token。单行最多159字符（不含终止符）；未知命令不改变配置。超长/非法控制字符整行作废，立即停止注入；RX ring overflow 清除已排队内容并丢弃到下一个换行边界，绝不执行截断前缀或尾串。

| 命令 | 行为 |
| --- | --- |
| `start` | 切为 Master，100ms后开始一轮：向当前地址写 `DE AD 00`，STOP，再读2B；每轮结束后间隔100ms |
| `stop` | 停止主机轮询、取消所有注入、释放GPIO/外设并转Idle；不会自动恢复 |
| `status` | 返回running/rounds、mode/address/hz、busy/lines、注入状态、计数、write/read status及原始SDK错误、最后2B读值 |
| `master <addr> <hz>` | 设置主机地址/速率，保持停止态，需再发start |
| `slave <addr> <hz>` | 设置参考从机并开始应答，保留256B表，偏移归零 |
| `stretch <us>` | Slave模式，每笔master-write第1个数据字节（偏移字节）推迟ACK；0关闭，范围0..200000us |
| `nack address <0或1>` | Slave模式；1禁用控制器的地址应答，形成地址缺席，0恢复；不是任意ADDRHIT ACK时序控制 |
| `nack data <index>` | Slave模式；每笔写的1-based字节编号，偏移字节为1，首表数据为2；0关闭，范围0..256。指定字节及之后内容不入表 |
| `hold <scl或sda或both> <ms>` | 仅空闲时进入GPIO故障，拉低指定线1..5000ms，到期释放并恢复已配置角色；旧master轮询不恢复 |
| `release` | 可在故障/忙态显式取消；释放线、清除stretch/NACK设置，恢复当前已配置角色，master轮询保持关闭；可重复执行 |
| `table fill <byte>` | 空闲时填充256B表，byte为0..255 |
| `table write <offset> <HEX>` | 空闲时写1..32B连续偶数字符HEX，不含空格，offset为0..255，片段不回绕 |
| `table read <offset> <length>` | 空闲时读1..32B；回复完整HEX，越界拒绝 |
| `help` | 简短命令说明 |

地址限制为 `0x08..0x77`；标准速率仅100000、400000、1000000Hz。`hz`是请求配置，不能代替实测。默认地址0x50、100000Hz，启动后Idle，不自动发事务。

角色、配置、表访问和新注入在外设忙/当前操作活动时返回 `ERR busy`；`stop/release`不受此状态限制，但仍遵守 CDC 请求/回复流控。若 host 停止读取 IN，应先读走回复，或关闭 DTR 触发独立的 stop 路径；不能把尚未解析的排队命令当作已执行取消。表为 EEPROM 语义：首写字节设偏移，后续写表并在0xFF回绕；读FIFO预取不计作已发送字节，实际偏移由完成时硬件DATACNT计算。HPM6700的DATACNT为8bit，status显式返回 `tx_count_bits=8` 和 `tx_datacnt_mod256_sum`：后者累加每笔读事务的DATACNT余数。单笔读达到256B后计数回绕，表偏移的模256语义仍成立，但该余数之和不能用于核算长事务的完整字节总数。table命令保留表内容，角色切换不擦表。两块对端之间的表独立；一个此固件仅提供一路物理I2C，不能冒充双路并发治具。

## 注入及取消语义

SDK HPM6700 `INTEN.BYTERECV` 明确关闭硬件auto-ACK。ISR先处理ADDRHIT、接收字节和CMPL；延迟ACK只记录起点和deadline，主循环到期写CMD_ACK，不在ISR或CDC内忙等。`last_stretch_us/max_stretch_us`记录软件观察的实际延迟，`ack_deadline_us`是请求截止时间，`poll_gap_max_us`记录轮询间隔。真实SCL低电平持续时间必须由仪器确认；不能声称精确10ms。

拉伸仅覆盖每笔write的首数据字节，不覆盖地址ACK或纯read。要验证正常10ms拉伸，先 `slave 0x50 100000`、`stretch 10000`，再由DUT发offset+data写事务；超过DUT预算可用 `stretch 80000`。若同一首字节被设置NACK，NACK优先，不再延迟ACK。收到CMPL、下一ADDRHIT、取消或硬件状态不再支持pending ACK时，旧ACK被作废；native覆盖的是状态机无late ACK，不证明在物理SCL被拉低时一定能形成STOP/reSTART。

GPIO故障先停I2C外设，再切PB11/PB10为GPIO open-drain。输出锁存器始终写0，拉低用output，释放用input，从不推高。到期或release先释放，再恢复I2C mux/配置；不自动发任何恢复脉冲。`hold sda 1000`可用于卡SDA、`hold scl 1000`用于卡SCL；主机应在自己的预算内失败。当前工具不按SCL边沿计数自动释放SDA，不能宣称“第9个脉冲释放” oracle 已具备；要验证9 clocks+STOP，需要逻辑分析仪及受控释放治具/人工时序。

主机旧周期路径已替换为非阻塞状态机，每轮总期限500ms，状态区分pending/ok/address_nack/data_nack/timeout/error/cancelled/not_run。成功write与read独立记录；write NACK后仍按旧序列读，仲裁错误/超时不重放已执行写入。失败重置仅本对端I2C，不证明外部卡线已恢复。

USB reset/disconnect、DTR关闭及RX overflow都会通知主循环stop，清除输入与活动注入，重连不启动旧角色/轮询。session loss在主循环处理前禁止CDC写入，即使连接已重新建立，也不会把旧会话prepare的回复送入新会话。CDC IN为1024B单消息非阻塞，FS/HS整包长度补ZLP；host不读IN时暂不处理更多普通命令，但deadline和loss事件仍执行。DTR关闭不等于DMA已取消，因此在途IN buffer直到完成或USB reset前不会复用。USB物理reset后的旧回调顺序依赖底层stack/controller，仍需上板拔插测试；不能从替身测试推断真实USB时序。

命令解析和全部回复格式化在全局中断锁外执行。锁内仅执行状态/HAL变更、复制status状态前缀或最多32B表快照；format只使用快照，不再读取HAL或实时peer。取出字节后若观察到session loss，则丢弃该次解析结果并stop。poll与配置重置路径仍使用全局中断锁，其完整target IRQ WCET尚未测量；这一拆分不等于已证明1MHz时序。

## 验证与未验证

native四组测试直接编译生产parser、peer/session、CDC buffer及`cdc_acm.c`（USB API替身）：覆盖范围/超长拒绝、32B片段、offset回绕、NACK字节不提交、10000us阈值及uint64时间回绕、busy拒绝、hold/release、取消后无late ACK、重配置失败释放、host不读IN、FS/HS ZLP、DTR/reset清理、连续RX overflow与CR/CRLF边界尾串不执行，以及状态/表在prepare后发生变化时回复快照仍一致。使用严格告警、ASan和UBSan；另执行RISC-V GCC严格语法检查和HPM6750EVKMini release编译链接。

未验证：物理FIFO/DATACNT、NACK/ACK、10ms低段及超时边界、1MHz连续通信、USB拔插、SDA/SCL实际开漏释放、波形边沿和板级复用。保存仪器原始波形、host/对端ELF SHA-256、板号、上拉、电平与接线后，才能据此验收S4；这些测试不关闭S5或S6真实设备验收。
