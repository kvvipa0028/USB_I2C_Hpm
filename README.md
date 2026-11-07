# USB 转 I2C 接口设备

面向 HPM6360（开发板 HPM6300EVK）的 USB 转 I2C 原型：固件在 `firmware/`，电脑端是 Rust workspace（`host/`）。当前固件只有 `CMD_PING` 与 `CMD_I2C`。PC 端已加入能力查询、显式端口/速率与双端口批次协议，以及明确标记的离线模型；扩展固件和实际时序仍待实现/上板。其余总线页面仍为本地演示。

*Prototype USB-to-I2C bridge for the HPM6360 on the HPM6300EVK. The on-wire protocol is PING and I2C master transfers only.*

产品目标（两路端口、每路可作主机或从机、C / Python SDK 等）写在 [PRD.md](PRD.md)，那是草案，不是本仓库已经交付的功能。

## 功能现状

| 项目 | 现状 | 依据 |
| --- | --- | --- |
| USB 桥固件 `firmware/usb_i2c` | 原型。快乐路径有一次台上记录；电脑端恢复路径**待上板** | 只做 I2C0 主机、100 kHz。提交 `ac4e73e` 的说明记载：EVK 桥与 EVKMini 寄存器表从机做过 PING、写、重复 START 读、缺席地址 NACK。仓库无该次原始日志。其后 GUI（PR #1、#2）与单桥恢复（PR #3）未再上板 |
| 电脑端库与 CLI `usb-i2c` | 代码已有。会话失效与重连行为**待上板** | 兼容 `list` / `ping` / `write` / `read` / `wr`，新增 `info`、`batch`、`--json`、`--offline`。能力按设备回复开放；超时或损坏回复使会话失效，不自动重试 |
| GUI `usb-i2c-gui` | 界面已合入（BusStudio 全模块页）。实机连接与 I2C **待上板** | 连接查询能力；旧固件仅 CH0/100k。显式选择离线模型后可操作两路独立 EEPROM；端口/频率/批次与结果来源可见 |
| I2C 扫描、总线恢复 | 本地演示 | 固件没有这些命令。按钮只往事务表插入标「演示」的示例行 |
| I2C 从机寄存器表 | 仅本窗口编辑；设备同步未实现 | 可编辑 256 字节表，关闭窗口后不保留。「下发到设备」「从设备读回」按钮禁用，固件没有对应命令 |
| 经典 CAN / CAN FD | 本地演示 | 支持协议切换、BRS、数据段速率和帧长度校验；接收生成示例帧，发送只追加 TX 记录，不发 USB |
| SPI、LIN、UART、PWM / 频率捕获、GPIO、AD 记录、ADS1015、EEPROM、Flash、UDS | 本地演示 | 侧栏有页面，数据留在本次窗口，不发 USB 命令，也不代表某块芯片或本设备的能力 |
| UDS Bootloader 真实执行路径 | P1 骨架：宿主 + 离线导入 + **模拟后端** | 按 [方案](docs/uds-bootloader-plugin-proposal.md) P1 实现任务状态、运行快照、事件归档和版本化后端接口；唯一后端是模拟后端，不打开总线、不发 UDS，结果不代表刷写。真实 CAN / ISO-TP / UDS 为 P2，方案 D01–D07 仍待确认。见 [UI_MERGE_NOTES.md](host/crates/gui/UI_MERGE_NOTES.md) |
| 硬件 I2C 主机自测 `firmware/i2c_proto`（T1.8） | 已上板（无单独日志） | 独立固件，对端是 SDK 从机示例。通过读数写在 [AGENTS.md](AGENTS.md)：`c0ffee18`、写/读成功、0 处不一致、缺席地址为 `status_i2c_no_addr_hit`（2003） |
| 从机寄存器表 `firmware/slave_proto`（T1.9） | 100 kHz 直连通过；400 kHz **未通过，待重测** | 独立固件，不经 USB。400 kHz 曾在接 10 kΩ 上拉、接线异常期间测试失败，正确接线下尚未重测。记录见 [docs/T1.9-slave-test.md](docs/T1.9-slave-test.md) |
| USB 批量吞吐 `firmware/usb_speed` 与 `usb-speed` | 工具已有；本仓库无实测报告，**待上板** | 与 I2C 桥不同固件。两边都使用测试号 VID:PID `1209:0001`，产品字符串分别为 `usb_speed` 和 `usb_i2c` |
| 双端口与角色切换、可调速率、地址扫描与总线恢复命令、上拉与 1.8–5 V、C / Python SDK、固件升级与防克隆 | 规划 | 见 [PRD.md](PRD.md)、[DEV_PLAN.md](DEV_PLAN.md)。PC 协议合同见 [usb-protocol.md](docs/usb-protocol.md)；双端口固件、`host/crates/sdk-c`、`sdk-py`、bootloader 和产品硬件仍未实现 |

「待上板」表示仓库里有对应代码或电脑端测试，但没有实体 HPM6300EVK 上的验收记录。PR #3 的单桥恢复、EEPROM 容量边界和 AD 演示计时只做过电脑端模型测试与界面测试，见 [docs/host-reliability.md](docs/host-reliability.md)。

CAN 页支持标准帧（11 bit）和扩展帧（29 bit）。经典 CAN 数据长度为 0–8 字节；CAN FD 只接受 0–8、12、16、20、24、32、48、64 字节，不自动补齐。启用 CAN FD 的 BRS 后可单独选择数据段速率，关闭 BRS 时使用仲裁速率。这些是本地演示的配置与校验规则，尚无 CAN / CAN FD 固件命令或硬件收发。

## 仓库结构

```
PRD.md                      产品需求草案 v0.2
DEV_PLAN.md                 开发计划草案 v0.2
AGENTS.md                   本机工具链、构建与烧录备忘
LICENSE                     MIT
功能清单_UTA0301对标.xlsx    对标清单（规划）
docs/
  host-reliability.md       单桥会话、失败不重试、演示页边界
  pwm-frequency-capture.md  PWM / 频率捕获合页说明
  images/                   PWM / 频率捕获浅色与深色预览图
  T1.9-slave-test.md        从机寄存器表上板记录
  T1.9-data/                上述测试的 OpenOCD 原文
firmware/                   目标板 HPM6300EVK（HPM6360），一次烧录一份
  app/                      串口 banner 与板载 LED，不含 USB / I2C 协议
  usb_i2c/                  当前 USB 桥（PING + I2C0 主机，100 kHz）
  i2c_proto/                T1.8 硬件 I2C 主机自测
  slave_proto/              T1.9 从机寄存器表（地址 0x50，256 字节）
  usb_speed/                USB 批量吞吐测试固件
host/                       Cargo workspace
  crates/usb-i2c/           库 + CLI（usb-i2c）
  crates/gui/               egui / eframe 上位机（usb-i2c-gui）
  crates/usb-speed/         吞吐测试客户端（usb-speed）
tools/
  env.sh                    HPM SDK、工具链、OpenOCD 路径
  flash.sh                  经板载 FT2232 烧录 ELF
  slave_test/               T1.9 对端主机（HPM6750EVKMini）
  siggen/                   EVK 上的 CDC I2C 对端原型；本仓库无上板记录
```

HPM SDK、RISC-V GCC 和 OpenOCD 在仓库外，默认放在 `~/hpm/`。

## 构建与运行

### 主机（macOS / Linux）

从仓库根目录开始，执行 `cd host` 后，本节后续命令均在 `host/` 下运行。依赖以 `Cargo.lock` 为准，请带 `--locked`。GUI 使用 egui 0.37，需要 Rust 1.95 或更新的 stable，以及图形环境。

macOS 还需安装 Xcode Command Line Tools（`xcode-select --install`）。主机程序使用本机 Rust 工具链；HPM SDK 和 RISC-V 工具链只用于后面的固件构建。

```sh
cd host
cargo build --release --workspace --locked
cargo test --workspace --all-targets --locked
```

启动上位机（窗口标题为 USB-I2C）：

```sh
cargo run --release --locked -p usb-i2c-gui
# 或直接运行已编好的二进制
./target/release/usb-i2c-gui
```

macOS 上的构建产物同样是 `target/release/usb-i2c-gui` 可执行文件。仓库尚未提供 `.app` 打包脚本或 macOS 安装包，手工生成的 `.app` 不属于仓库发布产物。

桥接器命令行（板子需已烧录 `firmware/usb_i2c`，产品字符串 `usb_i2c`）：

```sh
./target/release/usb-i2c list
./target/release/usb-i2c ping
./target/release/usb-i2c write 0x50 00 de ad
./target/release/usb-i2c read 0x50 16
./target/release/usb-i2c wr 0x50 16 00
```

这里的 `0x50` 是示例 7 位从机地址，长度 `16` 为十进制，数据字节按十六进制解析。写入示例按寄存器表从机的约定，以 `00` 为偏移；`wr 0x50 16 00` 表示先写偏移 `0x00`，再重复 START 读取 16 字节。实际使用时按目标器件协议调整地址和数据；`read` 是直接读，不先写寄存器地址。

无 EVK 时，在 GUI「设备连接」页点击「连接离线测试设备」。CH0/CH1 各有独立的 `0x50`、256 B EEPROM 模型，首个写字节为偏移，表尾回绕；其他地址返回 NACK。主机页可以选择端口与 10k–1M Hz 请求参数，双端口批次页一次提交每端口各一笔事务。速率只做参数校验，模型不产生 USB/I2C 信号，也不模拟 IRQ、拉伸和波形并发；断开后模型数据重置。

```sh
./target/release/usb-i2c --offline --json info
./target/release/usb-i2c --offline --port 1 --speed-hz 400000 wr 0x50 4 00
```

每次 CLI 调用都是新连接，离线内存不会跨进程保存。完整命令与机器可读验收入口见 [test-plan.md](docs/test-plan.md)。S1 三档固件构建及到板后的频率/连续传输验收见 [s1-bench-readiness.md](docs/s1-bench-readiness.md)；升级布局和安全设计边界见 [firmware-update-design.md](docs/firmware-update-design.md)。

吞吐工具配合已烧录 `firmware/usb_speed` 的板子：

```sh
./target/release/usb-speed list
./target/release/usb-speed rx --seconds 60 --transfer-size 1048576 --queue 8
./target/release/usb-speed tx --seconds 60 --transfer-size 1048576 --queue 8
```

`cargo test` 使用生产会话逻辑和 USB 完成类型的模型，不打开真实 USB 设备。Linux 上打开设备需要当前用户能 claim 该 USB 接口。

库与 GUI 的严格检查：

```sh
cargo clippy -p usb-i2c -p usb-i2c-gui --all-targets --locked -- -D warnings
```

### 主机（Windows）

在已安装 Rust 与 MSVC 目标的 Windows 上，进入仓库的 `host/` 目录，使用同一组 `cargo build` / `cargo test` / `cargo run` 命令。Release 图形程序是 `target\release\usb-i2c-gui.exe`（无控制台窗口）。

从 Linux 做的 Windows 检查只编译、不运行。安装对应 Rust target 后，在 `host/` 下执行：

```sh
cargo check --workspace --all-targets --locked --target x86_64-pc-windows-msvc
```

本仓库没有 Windows 安装包。固件里的 WinUSB / MS OS 2.0 描述符尚未在全新 Windows 上验收（**待上板**）。

### 固件

本节所有命令均在**仓库根目录**执行。若接着主机章节操作且当前仍在 `host/`，先执行 `cd ..` 返回根目录。

先准备 CMake、Ninja，以及仓库外的 HPM SDK v1.13.0、`riscv32-unknown-elf-gcc` 和 HPM 版 OpenOCD，再：

```sh
source tools/env.sh
```

`tools/env.sh` 的默认工具链路径是 macOS Intel 构建，本机通过 Rosetta 运行，外部动态库路径约束见 [AGENTS.md](AGENTS.md)。在 Linux 上先导出 `HPM_SDK_BASE` 和 `GNURISCV_TOOLCHAIN_PATH`，OpenOCD 路径不同时再设置 `HPM_OPENOCD_BIN`，然后 source。需要计时或吞吐时给 CMake 加上 `-DCMAKE_BUILD_TYPE=release`，否则 SDK 按 `-O0` 编译。

与当前主机关联的是 USB 桥：

```sh
cmake -S firmware/usb_i2c -B firmware/usb_i2c/build -GNinja \
  -DBOARD=hpm6300evk -DHPM_BUILD_TYPE=flash_xip -DCMAKE_BUILD_TYPE=release
ninja -C firmware/usb_i2c/build
tools/flash.sh firmware/usb_i2c/build/output/demo.elf
```

`tools/flash.sh` 走板载 FT2232。两块板同时连接时，按 [AGENTS.md](AGENTS.md) 用 USB location 区分；文档里的 `32-2.4` / `32-2.1.4` 只对当时的接线有效。T1.8、T1.9 和 `usb_speed` 的构建命令也在该文件。T1.9 的 `SLV_MODE` 与对端 `BUS_MODE` 必须一致，通过判据以 [docs/T1.9-slave-test.md](docs/T1.9-slave-test.md) 为准。

## 首次使用

浏览本地演示页面只需构建并运行 `usb-i2c-gui`，不要求连接硬件。进行真实 I2C 传输前，先完成主机程序构建，再按以下顺序准备：

1. 按固件章节构建 `firmware/usb_i2c`，通过板载 FT2232 烧录到 HPM6300EVK。
2. 使用连接 HPM6360 **USB0** 的板载 USB 接口，通过数据线接到电脑。FT2232 用于烧录和调试，仅连接该接口不会枚举出 `usb_i2c` 桥。
3. 接好 I2C0 与目标从机：EVK **J28[13] 为 SCL、J28[15] 为 SDA**，两端共地，并确认目标供电、逻辑电平和上拉接法匹配。当前桥固定 100 kHz。两板引脚与已测接线可参考 [T1.9 测试记录](docs/T1.9-slave-test.md)；该记录使用独立测试固件。
4. 在 `host/` 下执行 `./target/release/usb-i2c list`，确认列出桥设备，再执行 `./target/release/usb-i2c ping` 检查协议版本和固件标识。PING 成功只证明 USB 桥可通信，不代表 I2C 从机已应答。
5. 启动 GUI，刷新并连接设备，在 I2C 主机页填写目标的 7 位地址和数据，先执行单次读写。使用 CLI 时先断开 GUI 的设备连接，避免同时占用同一 USB 接口。

若 `list` 没有设备，先核对 USB0 数据线与固件类型：主机同时匹配 VID:PID `1209:0001` 和产品字符串 `usb_i2c`，`usb_speed` 固件不会出现在桥设备列表中。若设备已列出但无法打开，检查接口是否被其他进程占用，以及当前用户的 USB 访问权限。若 PING 成功而 I2C 返回 NACK，再检查从机地址、供电和 SCL / SDA 接线。

## 协议与安全边界

线上帧格式以 `firmware/usb_i2c/src/usb_i2c.c` 和 `host/crates/usb-i2c/src/proto.rs` 为准（协议版本 1，文件中标为原型）。评审用的协议文档尚未编写。

- 请求：`cmd, seq, addr, rsvd, wlen u16 LE, rlen u16 LE, wdata`
- 应答：`cmd, seq, status, rsvd, hpm_stat u32 LE, data`
- `CMD_PING`（`0x01`）：连接时同步会话，读回协议版本和固件标识（`usb_i2c 0.1`）。列出设备只枚举 USB，不发命令
- `CMD_I2C`（`0x10`）：I2C0、7 位地址、100 kHz。只写、只读，或写完后重复 START 再读。写、读各自 0–256 字节，两者同时为 0 时固件回非法请求。GUI 在送出前把寄存器地址拼进同一笔 `CMD_I2C` 的写入数据，仍然是这一条命令
- 应答状态：`0` 成功，`1` NACK，`2` 固件 I2C Timeout，`3` 非法请求，`4` 其它 I2C 错误
- USB：厂商类接口，批量 OUT `0x01`、批量 IN `0x81`。VID `0x1209`、PID `0x0001` 是 pid.codes 测试号，描述符注释写明不得用于出货

单桥会话（PR #3，细节见 [docs/host-reliability.md](docs/host-reliability.md)）：

1. 同时只保持一台桥。已有连接时必须先断开，再连接另一台或同一序列号。
2. 电脑端 USB 超时、传输错误、回复不匹配或损坏：该会话立即失效，不能再提交 I2C。界面清除连接状态。不自动重发任何 I2C 操作。完整且匹配的固件 NACK、固件 I2C Timeout 与这种超时是不同情况，见第 5 条。
3. 失败请求若含写入数据，写入结果未知。取消传输、断开或重新连接，都不能撤销目标器件上已经执行的写。
4. 重新打开只用有界的 `CMD_PING` 做同步和版本检查，不用 I2C 写入探测。同步不能确认上一笔写是否执行，也不能代替硬件复位。
5. 完整且序号匹配的 NACK 或固件 I2C Timeout 仍保留 USB 会话，但会停止界面循环。重新连接后循环不会自己恢复，需要手动再启动。
6. 刷新只重新枚举。未被程序观察到的物理断线，不能只凭界面状态判断（热插拔**待上板**）。

## 文档索引

| 文档 | 内容 |
| --- | --- |
| [PRD.md](PRD.md) | 产品需求草案：目标、范围、尚未冻结的决定 |
| [DEV_PLAN.md](DEV_PLAN.md) | 开发计划草案：阶段、目录规划、尚未落地的模块 |
| [docs/host-reliability.md](docs/host-reliability.md) | 单桥恢复、不自动重试、EEPROM / AD 演示页、测试与待上板项 |
| [docs/pwm-frequency-capture.md](docs/pwm-frequency-capture.md) | PWM / 频率捕获合页、本地演示交互、预览和硬件接入边界 |
| [host/crates/gui/UI_MERGE_NOTES.md](host/crates/gui/UI_MERGE_NOTES.md) | 哪些控件发 USB，哪些只在本窗口；页面与代码结构 |
| [docs/T1.9-slave-test.md](docs/T1.9-slave-test.md) | 从机寄存器表上板记录（100 kHz 通过，400 kHz 待重测） |
| [AGENTS.md](AGENTS.md) | 工具链、各固件的构建与烧录、T1.8 通过读数 |
| [功能清单_UTA0301对标.xlsx](功能清单_UTA0301对标.xlsx) | 与 UTA0301 的功能对标（规划） |

## 许可

本仓库以 [MIT 许可证](LICENSE) 发布，Copyright (c) 2026 赵建智。

上位机内置的中文字体子集（Noto Sans CJK SC）另按 SIL Open Font License 1.1 授权，见 [host/crates/gui/assets/fonts/OFL.txt](host/crates/gui/assets/fonts/OFL.txt)。
