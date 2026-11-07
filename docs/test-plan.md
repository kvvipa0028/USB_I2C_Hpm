# S0–S2 PC 测试与待上板矩阵

本文覆盖 `usb-i2c` 共用库、CLI、PC GUI 的离线模型与早期 HIL 骨架。离线运行不打开 USB，不验证 FIFO、ACK、时钟精度、真实双路并发或 Windows 驱动。S1/S2 的硬件出口保持未完成，直到有实际板卡与原始测量证据。

## CLI 合同

原有 `list`、`ping`、`write <addr> <hex...>`、`read <addr> <len>`、`wr <addr> <len> <hex...>` 语法保留。地址支持十进制或 `0x` 前缀；数据字节始终按十六进制解析。

新增全局选项可放在子命令前后：

- `--offline`：使用本进程内全新的双端口 EEPROM 模型，两个端口各自 256 B、地址 `0x50`、初值全零。进程结束后数据丢失；不会因真实 USB 失败自动切入此模式。
- `--port 0|1`：单笔传输的端口。旧固件仅支持 CH0。
- `--speed-hz <Hz>`：请求速率。离线模型检查 10kHz–1MHz 范围，不模拟时序；真实设备按实际能力接受或拒绝，不悄悄降速。
- `--json`：stdout 只输出一个 JSON 结果；错误同样有结构化结果和非零退出码。`--help` 保持 clap 的普通文本行为。

新增 `info` 返回固件信息和能力；`batch` 固定每端口一笔事务，两个端口分别用 `--ch0-addr/--ch0-write/--ch0-read` 与 `--ch1-addr/--ch1-write/--ch1-read` 描述。地址默认 `0x50`，写入字符串用空格分隔并加引号。每端口写/读上限各 240 B；至少有写或读。全局 `--speed-hz` 用于两笔批次事务，`--port` 仅用于单笔。该入口不提供任意命令列表、循环、跨步骤延时或自动重试。

```sh
cd host
cargo build -p usb-i2c --locked --offline
./target/debug/usb-i2c --offline --json info
./target/debug/usb-i2c --offline --port 1 --speed-hz 400000 wr 0x50 16 80
./target/debug/usb-i2c --offline --json batch --ch0-write "00 aa" --ch0-read 4 --ch1-read 4
```

JSON envelope v1：

```json
{
  "schema": "usb-i2c.cli",
  "schema_version": 1,
  "backend": "offline",
  "command": "read",
  "ok": true,
  "data": {"port": 0, "address": 80, "speed_hz": 100000, "written": 0, "data": [0]},
  "error": null
}
```

失败时 `error` 包含稳定的 `code`、可读 `message`、`session_invalidated` 和 `result_unknown`。错误码为 `invalid_argument`、`unavailable`、`operation_failed`、`batch_failure`；不通过错误文本猜测设备能力。参数解析失败的 `command` 为 `invalid`。成功退出 0，命令行参数错误退出 2，操作失败退出 1。批次有任一端口失败时退出 1，但 `data.outcomes` 保留两个端口各自的 `port/status/hpm_stat/data`；不能抹掉另一端已确认成功。USB/协议失败保留会话失效和结果未知语义，不重放原操作。

`info.data.capabilities` 给出 `extension_version`、`port_count`、`max_transfer`、`fixed_speed_hz`（当前/默认）、`min_speed_hz`、`max_speed_hz`、`supports_custom_speed`、`supports_port_transfer`、`supports_batch`、`batch_max_transfer`。这些值属于当前选择的 backend；离线能力不是已连接设备能力。

## 无设备检查

在仓库根目录执行：

```sh
cargo test --manifest-path host/Cargo.toml -p usb-i2c --all-targets --locked --offline
python3 -m unittest discover -s tools/hil -p 'test_*.py'
python3 tools/hil/run.py --mode offline --report outputs/hil/offline.json
```

HIL runner 支持 Python 3.9+，仅使用标准库，通过 CLI JSON 执行用例，不依赖尚未交付的 Python SDK。每次 CLI 只有一次尝试；进程超时标为结果未知，不重试。可用 `--cli <绝对路径>` 选择构建产物，`--timeout <秒>` 设置单进程期限。

模型用例的 oracle：

| 用例组 | 预期 | 证明范围 |
| --- | --- | --- |
| TC-CLI-01/02、TC-CAP-01 | 显式 offline 身份、两端口与模型能力、稳定 JSON | CLI/能力合同 |
| TC-MODEL-READ-CH0/CH1 | 新模型各端口读回全零 | 初值和端口访问 |
| TC-MODEL-WRAP-CH0/CH1 | 同一事务从偏移 0 写 1..255 后读 4 B，得到 `[0,1,2,3]` | EEPROM 自增、写入、回绕；每次 CLI 重置模型不会导致跨进程假读回 |
| TC-MODEL-SPEED-* | 六个目标速率接受并回报请求值；越界拒绝 | 参数范围，不是实际频率 |
| TC-MODEL-BATCH-ISOLATION | CH0/CH1 写不同内容，回绕后分别读到对应值 | 存储/结果隔离，不证明物理时间重叠 |
| TC-MODEL-BATCH-NACK | CH0 缺席地址 NACK，CH1 成功；外层失败但保留逐端口结果 | 结果归属 |
| TC-MODEL-*-REJECT/LIMIT | 非法端口、长度、空传输和批次过长被拒绝 | 参数边界 |
| TC-CLI-ARGUMENT-ERROR | 非法参数生成结构化错误并非零退出 | 自动化错误合同 |

库测试继续覆盖 v1 golden vectors、能力支持/不支持/损坏、会话失败、不自动重放、批次全量预检及离线状态行为；GUI 测试覆盖用户动作到同一 backend 的映射。CLI runner 无法证明同一连接内的全部生命周期，不能替代库的 Transport 模型测试。

报告 `usb-i2c.hil` v1 保存 CLI SHA-256、仓库提交、主机环境、UTC 时间、命令、退出码、原始 stdout/stderr、解析结果和耗时。`MODEL_PASS` 只累计 `model_pass`，`hardware_pass` 始终为 0，另列 `pending_hardware`。报告不得用于关闭 S1/S2 的硬件验收。

## 真实 USB 模式的当前边界

```sh
python3 tools/hil/run.py --mode usb --report outputs/hil/usb-smoke.json
```

当前 runner 的 USB 模式只做枚举、PING/能力查询，不向目标发送 I2C 事务。必须恰好枚举到一台桥；没有设备或多台桥时报告 `BLOCKED`、退出 2，不自动选择模型或某一台设备。正常得到 `USB_SMOKE_PASS` 仅表示协议冒烟通过，`hardware_pass` 仍为 0。后续硬件用例必须有板号、接线与对端 oracle 后再实现，不能仅把成功返回码当功能验收。

## S1/S2 待上板矩阵

| ID | 设置与操作 | 通过条件 | 必须保存的证据 |
| --- | --- | --- | --- |
| TC-S1-SLV-100K/400K/1M | release 构建的 slave_proto + slave_test；正确接线先100k再升速；随机读、当前地址读、重复 START、0xFF 回绕 | 每档原200轮冒烟及建议10min连续比对，错误数0；1MHz不得静默限速 | 板号/ELF hash/SDK/时钟、接线/上拉、9-word结果、表比对、原始日志、最大ISR延迟 |
| TC-S1-TIMING | 10k/50k/100k/400k/800k/1M，示波器或逻辑分析仪测SCL | 各档频率误差≤5%，高低电平时序符合设计 | 计算输入/寄存器、实测频率、波形及仪器设置 |
| TC-S2-COMPAT | 旧host/新固件、新host/旧固件、新host/新固件 | v1兼容；旧设备CH1/非100k等不支持操作明确拒绝；超时/损坏不得当legacy | 精确版本、原始请求/回复、结果 |
| TC-S2-MM-OVERLAP | 两独立对端、双路长事务批次 | 波形可见CH0/CH1活动重叠，两个结果和数据独立正确 | 双路同步采集波形、对端数据、批次结果 |
| TC-S2-FAULT-ISOLATION | 一端NACK或SCL卡低至timeout，另一端正常 | 正常端按自身deadline完成；错误不串端 | 两路波形、起止时间及逐端口结果 |
| TC-S2-USB-FAILURE | USB FS/HS整包边界、跨包/截断、迟到回复、拔线及重连 | 会话失效；未确认写入未知；无自动重放；重连不恢复循环 | USB/worker日志、对端实际执行计数、断线时序 |
| TC-S2-WINDOWS | 全新Win10/11 x64，app模式插入、PING和基本I2C | WinUSB免驱、准确身份/能力、基本读写正确 | OS版本、驱动信息、host/固件版本、运行日志 |

CH1 的控制器、引脚、IRQ及复用冲突必须先据实物核实。未核实前不得写死猜测映射；单对端、CH0→CH1回环和离线批次不能替代 M/M 双独立对端并发验收。标准模型不模拟真实 SCL 卡低、FIFO饥饿或电气状态。
