# S3–S6 端口控制协议 v1

日期：2026-11-07。本文冻结本轮实现合同，补充 `usb-protocol.md`，不改变现有 `0x02 CAPABILITIES` 的 20 B/version=1 格式。它是设计及实现约束，不是上板、电气或实时性通过证明。

## 公共约定

- 仍使用 8 B 公共请求/回复头、little-endian、单请求/单回复。下列命令请求头 `addr=0`，`wlen=负载字节数`，`rlen=表中成功回复数据字节数`；所有 reserved 位/字节为 0。
- 新命令沿用 status 0 OK、1 Nack、2 Timeout、3 BadRequest、4 I2cError，并增加 5 Busy、6 Unsupported、7 Conflict、8 WrongRole、9 ServiceDeadline。旧 0x10/0x11/0x12 仍只接受原 0–4 状态，旧命令在从机角色下返回 BadRequest。
- 错误回复无 data。Busy/Unsupported/Conflict/WrongRole/BadRequest 表示该请求未执行，保持会话和原有效状态。USB/格式/错序等错误作废会话；已提交的配置、写表、恢复、前端配置若未取得匹配回复，结果未知，不自动重放。查询不报告写入结果未知。
- `hpm_stat` 保留真实底层状态，纯协议/角色/能力拒绝为 0；不能用任意文字作为唯一机器可读状态。
- 请求 short-packet 填充仍按现有合同。新命令成功回复的实际整帧长若为 64 的整数倍，追加且只追加一个零字节；解析按已知长度剥离此唯一填充。错误回复仍为 8 B。
- 每端口最多一笔任务；USB OUT 在当前 IN completion 后才重新 arm。USB reset/disconnect 使旧 epoch 回复失效；不清角色/表，不重放请求，必要的表 mirror 恢复必须完成。

## 命令布局

共同 `PortQuery` 为 4 B：`port:u8, reserved[3]`。端口须小于 0x02 广告的 port_count。

| 命令 | 请求负载 | 成功回复数据 |
| --- | --- | --- |
| 0x03 CONTROL_CAPS | PortQuery | 48 B ControlCapabilities |
| 0x13 GET_PORT_STATE | PortQuery | 40 B PortState |
| 0x14 CONFIG_PORT | 32 B ConfigRequest | 40 B 实际应用后的 PortState |
| 0x15 TABLE_READ | 8 B TableRange | 16 B TableMeta + len B |
| 0x16 TABLE_WRITE | 16 B TableWriteMeta + len B | 16 B TableMeta |
| 0x17 PROBE | 4 B：port, address, reserved[2] | 4 B：port, address, present(0/1), reserved |
| 0x18 RECOVER | PortQuery | 8 B RecoveryResult |
| 0x19 FRONTEND_GET | PortQuery | 24 B FrontendState |
| 0x1A FRONTEND_SET | 12 B：port, source, pullup, reserved, expected_config_generation:u64 | 24 B FrontendState |

0x03 唯一降级条件为完整匹配的 8 B BadRequest、hpm_stat=0、无 data；此时仅辅助控制能力为 unsupported，不抹掉已经查询的原能力。超时/错序/未知版本/未知位/矛盾字段不得降级。

### ControlCapabilities（48 B）

| offset | 字段 |
| --- | --- |
| 0 | version:u8=1, port:u8, reserved:u16 |
| 4 | features:u32 |
| 8 | table_size:u16, max_table_len:u16 |
| 12 | table_service_us:u32 |
| 16 | stretch_min_us:u32, stretch_max_us:u32, stretch_default_us:u32 |
| 28 | transaction_min_us:u32, transaction_max_us:u32, transaction_default_us:u32 |
| 40 | control_deadline_us:u32 |
| 44 | frontend_options:u32 |

features：bit0 role/config，bit1 table，bit2 stretch/total timeout，bit3 probe，bit4 recover，bit5 frontend_get，bit6 frontend_set，bit7 table CAS；其余位拒绝。缺少相应能力的参数范围为 0，不能广告未实现功能。table 为 256 B，max_table_len=256。

本轮显式设计参数：table_service_us=2000；stretch 1000..100000 us，默认25000；transaction 1000..2000000 us，默认500000，且 transaction>=stretch；control_deadline_us=1500000。它们是待实际负载/WCET测量的验收目标和协议上限，不能凭编译或 Host 模型声称满足。

### ConfigRequest（32 B）及 PortState（40 B）

ConfigRequest：`version:u8=1, port:u8, role:u8, slave_address:u8, speed_hz:u32, stretch_timeout_us:u32, transaction_timeout_us:u32, flags:u32, reserved:u32, expected_config_generation:u64`。flags bit0 表示启用 generation 比较，其他位为 0；未启用时 expected=0。

PortState：`version:u8=1, port:u8, role:u8, activity:u8, slave_address:u8, reserved[3], speed_hz:u32, calculated_speed_hz:u32, stretch_timeout_us:u32, transaction_timeout_us:u32, config_generation:u64, table_generation:u64`。

- role：0 Master、1 Slave。activity：0 Idle、1 Busy、2 Fault。
- speed_hz 是已应用默认配置；master 允许10k..1M且必须有合法时序解，slave仅100k/400k/1M。calculated_speed_hz 为寄存器计算值，不是测量值。0x11/0x12 的每笔 speed_hz 覆盖仅作用该笔，不修改已应用默认配置。
- slave_address 只允许0x08..0x77。所有字段及可选 generation 在任何副作用前验证；忙端口立即 Busy，不无限等待STOP。
- 仅重置/重配所选 I2C 控制器；成功才增加 config_generation。失败保持/恢复旧有效配置，不能复位另一端口或整机。进入/重新配置从机时 offset归零，表内容保留。

### TableRange / TableMeta

TableRange（8 B）：`port:u8, reserved:u8, offset:u16, len:u16, reserved:u16`。

TableMeta（16 B）：`port:u8, reserved:u8, offset:u16, len:u16, reserved:u16, generation:u64`。

TableWriteMeta 与 TableMeta 同布局，但第二字节为 flags（bit0 expected_generation有效），最后8 B为expected_generation。未启用CAS时expected=0。

- 仅 Slave 角色允许 PC 表访问；len=1..256，offset+len<=256，采用u16/更宽中间运算。PC片段不回绕，不改变I2C current offset；外部I2C EEPROM在0xFF回绕。
- PC read 为一个提交点的一致快照，返回该点的 generation。PC write 整个片段在一个提交点可见，完成后返回新 generation；同址写按提交顺序覆盖。外部每个已ACK数据字节和每笔PC片段提交分别增加一次u64 generation；offset-only不增加。
- 新发起的外部读可见最新已提交内容；正在进行的外部流式读可能跨PC提交，FIFO已经预取的字节不会被撤回。外部整笔读不承诺单一版本快照，此语义须用于HIL oracle。
- 表和generation在角色切换/USB重连后保留，MCU reset后归零；无NVM持久化。CAS冲突不提交。T_PC适用于满足前提的非冲突请求，不能用反复Busy/Conflict代替服务完成。

实现采用 active/mirror 两份表。平时ISR字节提交写两份；PC snapshot在常数时间短IRQ锁内冻结mirror并锁存generation，锁外拷贝。随后进入RESYNC，ISR恢复双写，逐字节在短IRQ锁中把active复制到mirror。PC write短锁设置mirror排除区间，锁外写入该片段，短锁内swap+取消排除+增加generation为线性提交点，再RESYNC。RESYNC结束前不接下一PC表操作。不得关IRQ复制整表；不得用volatile代替并发协议。所有IRQ锁都须记录并实测WCET，操作数有界不等于墙钟时间已通过。

### Probe、scan 与 recovery

- PROBE仅允许master及0x08..0x77，以已应用默认速率发START+地址+STOP，不发寄存器/数据。ACK为present=1；NACK可返回present=0，或公共Nack无data（host统一为不存在）。其他错误立即中止扫描，不当不存在。
- host scan按0x08..0x77逐地址probe；全程上限5s，取消在地址之间生效。GUI取消必须走独立共享取消标志，不能排在正在scan的worker队列后。返回地址集合及 complete/cancelled/error，部分扫描不能显示为完整成功。
- stretch_timeout_us检测真实SCL低期间；硬件byte progress证明出现过时钟，可重置低段计时以防高速低电平采样混叠。整笔transaction deadline独立按真实时间计算，不能使用固定poll次数，不能偷换为任意no-progress等待。采样分辨率和误差须上板记录。
- recovery仅master且无活动任务；GPIO输出锁存0，以output拉低、input释放，绝不主动推高。发送9个SCL脉冲，再形成STOP并检查SCL/SDA实际释放。单次释放等待受该端口stretch上限约束，总时限<=control_deadline_us；结束必须释放线并恢复所选端口mux/原配置。
- RecoveryResult：`port:u8, clocks_sent:u8, initial_lines:u8, final_lines:u8, reason:u32`。line bits 0=SCL high、1=SDA high；其他位0。reason 0成功、1SCL卡低、2SDA卡低、3总期限、4重新初始化失败。公共status=OK仅表示命令已完成并返回诊断；reason非0不能在GUI显示成功或让CLI返回exit0。前置Busy/Unsupported/WrongRole等仍为公共错误、无data。不得自动重试写入。
- 新host主机传输（含PROBE）等待上限使用设备广告transaction_max_us+500000us；控制命令使用control_deadline_us+500000us。legacy和只有0x02的旧扩展固件仍使用原1s合同。Scan的5s限额允许最后一个已开始probe的有界清理和USB余量，取消同样不宣称即时撤销在途事务。

### 前端边界

source：0 External、1 Internal1V8、2 Internal3V3、3 Internal5V；pullup：0 Off、1 Ohm4K7、2 Ohm2K2。frontend_options低4位为source支持位、bit8..10为pullup支持位，其余必须0。

FrontendState：`version:u8=1, port:u8, source:u8, pullup:u8, valid_flags:u16, fault:u16, measured_vref_mv:u32, reserved:u32, config_generation:u64`。valid_flags bit0表示有效电压测量；没有该位时电压值不得被显示为实测结果。SET使用expected_config_generation避免旧草稿覆盖新配置。

当前无前端小板/EVT资料的EVK必须广告frontend能力为0，GET/SET返回Unsupported；不得填0mV或固定值冒充测量/成功。真实ADC/GPIO/电源/保护映射只能来自经核实的电路资料，不能靠GUI或引脚猜测补全。原理图、PCB、BOM、1.8–5V波形、ESD及功耗仍为独立待交付/待实测项。

## Rust API 协作接口

新增 `usb_i2c::control` 模块，公开 `ControlCapabilities`（各features对应supports_*布尔字段及上述范围）、`PortRole::{Master,Slave}`、`PortActivity::{Idle,Busy,Fault}`、`PortConfig`、`PortState`、`TableSnapshot { offset:u16, data:Vec<u8>, generation:u64 }`、`TableCommit { offset:u16, len:u16, generation:u64 }`、`RecoveryResult`、`FrontendConfig`、`FrontendState`、`ScanResult`。所有用于GUI事件的值实现Debug/Clone；需要比较的配置/状态实现PartialEq/Eq。

`Device`新增以下方法（保持旧接口）：

```rust,ignore
control_capabilities(&self) -> &[ControlCapabilities];
port_state(&mut self, port:u8) -> Result<PortState, DeviceError>;
configure_port(&mut self, port:u8, config:&PortConfig,
               expected_generation:Option<u64>) -> Result<PortState, DeviceError>;
table_read(&mut self, port:u8, offset:u16, len:u16) -> Result<TableSnapshot, DeviceError>;
table_write(&mut self, port:u8, offset:u16, data:&[u8],
            expected_generation:Option<u64>) -> Result<TableCommit, DeviceError>;
probe(&mut self, port:u8, address:u8) -> Result<bool, DeviceError>;
scan(&mut self, port:u8, cancelled:impl FnMut()->bool) -> Result<ScanResult, DeviceError>;
recover(&mut self, port:u8) -> Result<RecoveryResult, DeviceError>;
frontend_get(&mut self, port:u8) -> Result<FrontendState, DeviceError>;
frontend_set(&mut self, port:u8, config:&FrontendConfig,
             expected_generation:u64) -> Result<FrontendState, DeviceError>;
```

`PortConfig`字段为`role, slave_address, speed_hz, stretch_timeout_us, transaction_timeout_us`。`PortState`字段为`port, config, activity, calculated_speed_hz, config_generation, table_generation`。`ScanResult`字段为`addresses:Vec<u8>, complete:bool, cancelled:bool, error:Option<DeviceError>`；参数拒绝可为外层Err，逐地址失败保留部分集合及原DeviceError，不丢失session_invalidated/result_unknown。所有新worker事件保留DeviceError，不能提前压成String丢状态。

offline仍显式选择、绝不因USB失败自动切换。模型分别保存两路外部EEPROM及设备从机表/角色/配置，不能把外部对端模型等同于本设备从机表。设备从机角色拒绝主机传输；表操作/配置/CAS/probe/recover模型只验证PC合同，不产生波形，不提供前端伪测量。
