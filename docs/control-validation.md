# S3–S6 控制实现与验收边界

本轮交付端口控制协议、双端口固件、Rust API、CLI、GUI 和测试入口。协议字段及一致性语义以 [control-protocol-v1.md](control-protocol-v1.md) 为准。源码、Host/native 测试、target 编译链接和上板测量是不同证据层；本轮没有连接 EVK，不能关闭硬件验收。

## 实现范围

| 步骤 | 已接入的软件路径 | 未完成的出口 |
| --- | --- | --- |
| S3 | 独立主从角色/配置、每端口 256 B 从机表、PC 快照与 CAS 片段提交、实际从机 ISR、GUI 表读写 | M/S、S/M、S/S 外部对端测试；1 MHz 持续流量下表服务期限和 IRQ WCET |
| S4 | 10k–1M 主机时序计算、真实时间总期限/SCL 低段期限、无数据 probe/扫描、有界 GPIO recovery、双路批次状态机 | 示波器频率/高低时序、10 ms 拉伸、卡线恢复波形、物理并发与故障隔离 |
| S5 | 能力/状态/配置 wire contract、Rust/CLI/GUI 入口；EVK 与 offline 显式 Unsupported | 原理图、PCB、BOM、ADC/GPIO/保护映射、1.8–5V、上拉、ESD、功耗，均未完成 |
| S6 | 配置草稿与设备回读分离、代次比较、表快照、扫描取消/部分结果、恢复失败状态、会话失效不重放 | USB 实机 GUI、Windows 驱动与物理断连验收；本地模型不能替代 |

不修改 `PRD.md`、`DEV_PLAN.md` 或功能对标表中的验收勾选。C/Python SDK、产品 Bootloader 和真实 UDS 不在本轮实现范围。

## 模块和边界

- `firmware/usb_i2c/src/bridge_protocol.c`：生产报文解析、全帧/保留位/批次预检、控制分发与响应；native 测试直接编译此文件。
- `firmware/usb_i2c/src/i2c_port.c`：每端口主机状态机、slave ISR、角色/时序与表提交；native 寄存器替身测试直接编译此文件，不能证明寄存器的真实硅片行为。
- `firmware/common/`：时序、真实时间 deadline、表 active/mirror 一致性和 recovery；表复制在锁外，RESYNC 逐字节短锁，不在关 IRQ 区复制整表。
- `firmware/boards/evk/`：依据 SDK v1.13.0 的 EVK 映射；CH0=I2C0 PC13/PC14，CH1=I2C3 PC11/PC12。CH1 连接器引出、板级冲突与实际接线仍需实物核对。未推断产品前端引脚。
- `host/crates/usb-i2c/src/control.rs`、`bridge/control.rs`、`device.rs`：严格 codec、typed status、能力限制、配置和表 CAS、失败会话作废；不将 USB 故障降级为 legacy 或 offline。
- `host/crates/gui/src/worker.rs`、`i2c/`：同一 worker 控制实际 backend，扫描取消标志独立于队列；断连后清除设备确认状态，旧草稿不自动下发。

`g_bridge_irq_lock_ticks_max` 只记录关 IRQ 后的锁内主体，未覆盖计时读取、统计更新和 restore 的完整开销，不能独立作为 IRQ WCET。上板应使用包围 disable 至 restore 全区间的 GPIO/trace 测量。

每笔 `0x11/0x12` 的速率覆盖不改变持久默认配置。表与代次在角色切换/USB 重连后保留，MCU reset 后归零；offline 新连接重建模型。外部 EEPROM 模型与设备从机表是两个不同对象。

## 可复现本地检查

从仓库根目录执行：

```sh
cmake -S firmware/tests -B /tmp/usb-i2c-native -G Ninja
cmake --build /tmp/usb-i2c-native
ctest --test-dir /tmp/usb-i2c-native --output-on-failure

source tools/env.sh
cmake -S firmware/usb_i2c -B firmware/usb_i2c/build -G Ninja \
  -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
cmake --build firmware/usb_i2c/build

cargo test --manifest-path host/Cargo.toml --workspace --all-targets --locked
cargo clippy --manifest-path host/Cargo.toml -p usb-i2c -p usb-i2c-gui \
  --all-targets --locked -- -D warnings
cargo build --manifest-path host/Cargo.toml --workspace --release --locked
python3 -m unittest discover -s tools/hil -p 'test_*.py'
python3 tools/hil/run.py --mode offline --cli host/target/release/usb-i2c \
  --report outputs/s3-s6-offline.json
```

C 测试启用严格警告和 ASan/UBSan。Host 测试覆盖生产 codec/session/worker/UI 行为；CLI 每次启动新模型，跨请求状态由同一 Device 会话的 Rust 测试验证，不伪造 CLI 跨进程持久化。`.github/workflows/host.yml` 同时运行 native C 与 macOS/Windows Host 检查。

Windows `cargo check --target x86_64-pc-windows-msvc` 只证明编译；在 ARM64 Windows VM 运行 x64 EXE 只能证明该兼容运行环境，不等于 x64 物理机或 VM 内原生编译。具体本轮测试计数和产物关联保存在本地验证报告中。

## 必须上板的验收矩阵

| ID | 设置与操作 | 通过条件与原始证据 |
| --- | --- | --- |
| S3-ROLE | M/M、M/S、S/M、S/S；空闲切角色、忙时切换、地址及速率改变 | 只影响所选控制器；忙时 Busy 且旧配置不变；切换保留表、offset 按合同归零。保存对端数据与双路波形 |
| S3-TABLE | PC 1/16/256 B 快照与片段写；外部随机写、回绕、短事务、重复 START、READ→WRITE | 快照对应单个 generation；PC 片段原子可见；CAS 冲突不提交；外部已预取字节允许旧版本，不能使用整笔读快照 oracle |
| S3-SERVICE | 两路 1 MHz 持续外部流量下反复 PC 读写 | 满足前提的非冲突请求 T_PC≤2000 us，不能反复 Busy 代替完成；记录 `g_bridge_table_service_us_max`、`g_bridge_irq_lock_ticks_max`、实时时钟和 IRQ 波形，核实 WCET |
| S4-SPEED | 10k/50k/100k/400k/800k/1M 以及边界自定义速率 | 实测频率误差≤5%，tHIGH/tLOW 合规；同时保存请求值、计算值、寄存器和测量值，不能把 calculated_speed_hz 当实测 |
| S4-STRETCH | 对端拉伸10ms、超过配置上限、SCL持续低、SDA低但SCL高 | 10ms在允许预算内成功；低段与总deadline分别有界；记录采样误差和失败时实际已执行字节数 |
| S4-SCAN | 接入已知地址集合，注入NACK/超时，操作取消 | 只发 START+地址+STOP；NACK当缺席，其他错误保留partial并中止；取消在probe边界生效，不声称撤销在途事务 |
| S4-RECOVERY | 正常线、SCL卡低、SDA卡低、9脉冲后释放、期间USB断连 | 开漏拉低/释放，无主动推高；实际9 clocks+STOP；reason与实际线状态一致，所有路径释放线并恢复mux，不重放写入 |
| S2/S4-ISOLATION | 两独立对端长事务，一端NACK/拉伸超时、另一端正常 | 波形证明活动重叠与故障隔离；各自deadline与结果正确，轮询交替不等于物理并发 |
| S6-USB | FS/HS边界长度、在途拔线、坏回复、旧epoch IN回调、重连 | 只有IN完成才rearm；旧回复丢弃；配置/写表/恢复结果未知时不自动重放；重连循环关闭、先读状态 |
| S5-ELECTRICAL | 经确认的前端资料和EVT已具备后 | 按PRD完成1.8/2.5/3.3/5V、上拉和VREF精度、保护/ESD/功耗；当前Unsupported不能算通过 |

每项保存板号、接线/上拉、电源、host/ELF SHA-256、SDK/时钟、原始日志、仪器设置及波形。没有设备时 USB runner 应报告 `BLOCKED`，`hardware_pass=0`；上述验收项保持 PENDING。
