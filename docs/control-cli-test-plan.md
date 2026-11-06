# S3–S6 CLI 与测试入口

本文件说明控制 CLI 的操作合同和可复现验证，协议布局以
[control-protocol-v1.md](control-protocol-v1.md) 为准，硬件验收以
[control-validation.md](control-validation.md) 的矩阵为准。Host/offline PASS
仅证明软件合同；频率、拉伸、recovery 波形、表服务期限和电气能力需要独立实测。

## 命令与状态

所有命令保留 `--json`、`--offline`、`--port` 和 `--speed-hz`
全局参数；旧 `list/ping/info/write/read/wr/batch` 语法保持兼容。
未传 `--offline` 就连接 USB；连接失败不会切换模型。

| 命令 | 参数与含义 |
| --- | --- |
| `info` | 返回原能力、每端口 `control_capabilities` 和来源；旧固件的控制能力明确为 unsupported |
| `port-state` | 读取实际角色、默认速率、超时、activity、配置/表 generation；`calculated_speed_hz` 是寄存器计算值，`measured_speed_hz=null` |
| `port-config` | `--role master\|slave` 必填；`--slave-address`、全局 `--speed-hz`、`--stretch-timeout-us`、`--transaction-timeout-us` 构成完整配置，`--expected-gen` 可启用配置 CAS |
| `table-read OFFSET LEN` | Slave 角色的一致快照；十进制或 `0x` offset，LEN 为十进制；长度 1–256，片段不回绕 |
| `table-write OFFSET HEX...` | Slave 角色原子片段提交；每字节使用十六进制，`--expected-gen` 可启用表 CAS；回复新 generation |
| `scan` | 在 Master 角色按 0x08–0x77 做无数据 probe；保留部分地址、complete/cancelled/error，不把中途故障当作完整成功 |
| `recover` | 有界恢复并报告实际诊断；公共命令完成不代表恢复成功，`reason!=0` 返回非零退出码 |
| `frontend-status` | 读取支持的前端状态；`voltage_valid=false` 时电压为 null，不显示伪测量 |
| `frontend-config` | `--source external\|internal1v8\|internal3v3\|internal5v`、`--pullup off\|4k7\|2k2` 和 `--expected-gen` 均必填 |

`port-config` 是完整配置提交，不是仅修改指定字段的 patch。省略的字段使用
CLI 默认值：地址 0x50、速率 100000 Hz、stretch 25000 us、transaction
500000 us。需要保留其他字段时，先读 `port-state`，显式填入回读值与
`--expected-gen`。Master 速率为 10k–1M，Slave 只接受 100k/400k/1M。

JSON envelope 保持 `schema=usb-i2c.cli`、`schema_version=1`、明确 backend、
command、ok、data、error。控制错误包含 `status`、`status_code`、`hpm_stat`、
`session_invalidated` 和 `result_unknown`。参数解析/前置校验错误退出码为 2，
操作失败为 1，成功为 0；未知结果不得自动重试写入。扫描取消在逐地址边界
处理，CLI 当前没有交互式取消输入；GUI 独立取消标志和 API cancel callback
由各自测试覆盖，不声称杀死 CLI 进程会返回部分 JSON。

## 使用示例与模型边界

```sh
host/target/release/usb-i2c --json --offline info
host/target/release/usb-i2c --json --offline --port 1 port-state
host/target/release/usb-i2c --json --offline --port 1 --speed-hz 400000 \
  port-config --role slave --slave-address 0x51 \
  --stretch-timeout-us 30000 --transaction-timeout-us 600000 --expected-gen 0
host/target/release/usb-i2c --json --offline --port 0 scan
host/target/release/usb-i2c --json --offline --port 0 recover
```

每个 CLI 进程会重新创建 offline 模型，所以以上示例相互独立；不能先启动
offline `port-config`，再用另一个进程声称读到了同一设备表。
`main.rs` 的同会话 dispatch 测试保留一个 `Device::offline()`，验证配置回读、
0xFF 边界表写、CAS 拒绝无副作用、角色切换保表与双端口隔离。该模型中的
外部 EEPROM 与本设备 Slave 表是不同对象。

真实设备的表操作顺序为：先读取 `port-state`，用完整配置及回读 generation
进入 Slave，读取 `table-read 0 256` 的 generation，再执行例如
`table-write 255 aa --expected-gen <回读值>`，最后回读核对。外部 I2C 写入也会
推进表 generation，Conflict 后必须重新读取，不自动覆盖。此流程会改变设备
状态，未加入默认 USB smoke runner。

EVK 和 offline 都没有受支持的前端电气实现；frontend GET/SET 应明确返回
Unsupported。它们不能证明供电选择、上拉、电压精度或保护有效。

## 自动验证分层

| 层 | 覆盖 | 不代表 |
| --- | --- | --- |
| Rust CLI 8 项 | 原语法、参数拒绝、JSON 确定性、同会话配置/表/CAS、partial scan、恢复失败、无效电压 | 真实 USB、寄存器与外部总线 |
| Python adapter/runner 18 项 | 模式/退出码一致性、不重试、typed diagnostics、partial/recovery/voltage 拒绝假成功、只读 USB 能力矩阵、缺设备 BLOCKED | 设备行为或 Python mock 以外的硬件证据 |
| offline CLI 51 项 | 23 项原传输/批次用例及 28 项控制命令、双端口、非法参数/角色/CAS/Unsupported 用例 | 跨进程模型持久化、真实时间和电气功能 |
| USB smoke | 唯一设备枚举、info、按能力读取 port-state/frontend-status；不发目标 I2C 事务或修改配置 | 扫描/recovery 波形、表实时性、角色切换、物理并发 |
| 物理矩阵 | `run.py` 列出 S1–S6 未执行项；具体设置和 oracle 见验收文档 | 在没有原始测量时关闭任何硬件项 |

USB runner 遇到零台或多台设备返回 BLOCKED；协商能力缺失/矛盾返回 FAIL。
支持的状态查询成功计入 `usb_protocol_pass`；依据能力在 Host 拒绝的 Unsupported
计入独立的 `capability_gate_pass`，不计为设备执行命令。任何查询失败立即结束
本次 smoke。所有模式的 `hardware_pass=0`，物理用例保持 `NOT_RUN`。

```sh
cargo test --manifest-path host/Cargo.toml -p usb-i2c --bin usb-i2c --locked
cargo clippy --manifest-path host/Cargo.toml -p usb-i2c --all-targets --locked -- -D warnings
cargo build --manifest-path host/Cargo.toml -p usb-i2c --release --locked
python3 -m unittest discover -s tools/hil -p 'test_*.py' -v
python3 tools/hil/run.py --mode offline --cli host/target/release/usb-i2c \
  --report outputs/s3-s6-cli-validation-20261107/offline.json
python3 tools/hil/run.py --mode usb --cli host/target/release/usb-i2c \
  --report outputs/s3-s6-cli-validation-20261107/usb-smoke.json
```

2026-11-07 本机执行结果：CLI Rust 8/8、Python 18/18、offline 51/51 PASS，
CLI Clippy 和 release build PASS。USB 枚举返回零设备，runner exit 2 / BLOCKED；
`usb_protocol_pass=1` 仅为枚举成功，`hardware_pass=0`。本地原始日志、JSON 报告
和产物指纹位于 `outputs/s3-s6-cli-validation-20261107/`。本次子范围未执行
Windows VM 或实机验证，也未对此前 Windows 报告作新版本通过推断。
