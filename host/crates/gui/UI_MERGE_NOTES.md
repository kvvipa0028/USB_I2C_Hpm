# 上位机 UI 融入说明（BusStudio × Magpie）

范围：只改 `host/crates/gui`。USB 协议、固件、`usb-i2c` crate 都没动。

## 改了哪些 UI

- **壳**
  - 顶栏设备条：品牌、设备弹出字段（状态圆点 + 序列号 + USB 速率 + chevron，替代原生 ComboBox）、固件徽章（从 PING 回复解析出固件和协议版本）、刷新、连接/断开。
  - 底栏状态条：连接状态、空闲/进行中（转圈）、最近结果的软徽章（成功绿、失败红），高度固定，不挤动页面。
  - 左侧导航保留原来的 Magpie 样式（选中项白底加小阴影，文字不染 accent）。设备连接项带状态圆点，SPI/PWM 标「未实现」。
- **设备连接页**：设备列表卡（逐行选择，标出已连接），连接信息（状态、序列号、固件、协议版本、端口、速率、单次上限）。
- **I2C 工作台**：页头右侧是「主机 | 从机」分段控件。
  - 主机：左栏「接口配置」（端口、速率由固件固定，只显示；目标地址、读取长度可填，输入有误时就地提示）和「总线工具」；右栏「读写事务」（HEX 输入、实时字节数、写入/读取/写后读）、「结果」（hex dump，可复制 HEX）、「事务记录」（本次会话，最新在上，最多 200 条，点行查看）。
  - 从机：左栏「从机配置」（端口、角色、从机地址、256 B / 1 字节偏移）和「与设备同步」；右栏 256 B hex 网格编辑器：点格子选中，键入两位十六进制改值，方向键移动；改过的格子标 accent 软底；带 ASCII 列和检查器（偏移、值、十进制、二进制、ASCII、改为…）；全部 00 / 全部 FF / 递增 / 还原 / 复制 HEX。
- 窗口默认 1200×800，最小 900×600；窄于断点时两栏改为上下排；内容最宽 1200 居中。

## 仍是占位（不发任何命令）

| 位置 | 状态 | 依赖 |
| --- | --- | --- |
| 扫描地址、总线恢复 | 按钮 disabled，悬停说明「协议未实现」 | 固件新增命令（PRD FR-MST-05/06，DEV_PLAN T2.5） |
| 从机：下发到设备、从设备读回 | 同上；寄存器表只在本窗口编辑，初值全 0（同 slave_proto 复位后） | 从机协议（FR-SLV、T2.20） |
| 端口选择、主从切换、调速 | 只读显示 I2C0 · 主机 · 100 kHz | 固件 |
| SPI / PWM | 侧栏保留入口，页面写明固件未实现 | 第二批 |

## 与 BusStudio / Magpie 的取舍

- 主色沿用现仓 `tokens()` 的 Magpie indigo（浅 `#4f46e5` / 深 `#a5b4fc`），**没有**用 BusStudio 稿的 `#6657db`。accent 只给焦点、打开中的弹出字段、选中格子和改过的格子。
- 主按钮用墨色（fg），次按钮白底描边，工具按钮无底（Magpie 按钮层级）。状态一律用圆点和软徽章，没有彩色左边条。
- 从 BusStudio 拿的是结构：顶栏设备条、页脚状态、「配置 | 工作区」两栏、面板头 + 细线、字段行、事务表、EEPROM 网格的观感。没有搬它的横向页签、完整三步向导和演示数据。
- BusStudio 的 HTML 没有进仓库。
- Token 补了 Magpie 的 `line2 / fg2 / faint / accent_soft / green(-soft) / red-soft / amber-soft / ink`，深浅色仍跟随系统。

## 协议与行为

- 协议没改，线上仍只有 `CMD_PING`（连接时）和 `CMD_I2C`。
- worker 保留 `Job` / `Event` 模式；新增 `Job::Disconnect` 只在本机释放 USB 接口，不向设备发东西。
- **行为修正**：原来「读取」会把写入框的数据一起发出，有数据时实际变成写后读。现在「读取」与 CLI 的 `read` 一致，只读，不看写入框。
- 状态文案细化（「正在连接 xxx…」「已断开」等）；传输成功/失败文案沿用原来的。
- 已打开的设备不在刷新结果里时：顶栏字段仍显示该序列号并标「不在列表」，圆点变红，「断开」一直可点，主机读写按钮禁用（`Devices::can_disconnect` / `can_send`）。
- 改连另一台设备失败时保留原来打开的设备，新接口 PING 成功后才替换。

## 中文字体

- 内置 `assets/fonts/NotoSansSC-UI.otf`：Noto Sans CJK SC Regular 按 `src/` 里实际用到的字（加常用中文标点）裁成子集，约 83 KB / 241 个字符；许可证 SIL OFL 1.1，见同目录 `OFL.txt`。
- 加中文文案后重跑 `assets/fonts/subset.sh`（需要 fonttools 和 Noto Sans CJK）。测试 `fonts::tests::subset_covers_ui_text` 扫描全部源码，有字不在子集里就失败。
- 子集排在 egui 自带字体之后，英文和数字仍用 egui 字体。子集外的字（例如 Windows 返回的中文系统错误、用户粘贴的文本）交给 eframe 默认开启的系统字体回退。
- 缺口：只内置了 Regular 一个字重，没有粗体（egui 的 `strong()` 本来就只加深颜色，中英文一样）；中文 Regular 比 egui 的英文字体 Ubuntu Light 略重。

## 验证

- `cargo check -p usb-i2c-gui`、`cargo test -p usb-i2c-gui`（13 个测试）、`cargo clippy -p usb-i2c-gui --all-targets -- -D warnings`、`cargo fmt --check` 全部通过。
- egui 0.37 需要 Rust ≥ 1.95；系统自带的 cargo 1.85 会在依赖解析阶段失败，要用 rustup 的 stable。
- 在 Linux（Xvfb，wgpu GL 后端）跑过窗口：深/浅色，设备页、主机页、从机页，900 px 窄窗口，网格键入，设备弹出菜单，以及注入假数据后的结果卡和事务表。
- **没有接真机测试**：连接、PING、读写走的是原来的 `Bridge` 代码，改动只在 UI 侧，但还需要在 HPM6300EVK 上过一遍。
- 工作区里 `cargo clippy -- -D warnings` 在 `usb-speed` 上有 2 处报错（`chunks_exact` 用常量块长），是新版 clippy 引入的，与本次改动无关，没有动。

## 代码结构

```
src/main.rs        入口和窗口尺寸
src/app.rs         App 状态、事件处理、壳的组装（顶栏 / 底栏 / 侧栏 / 页面）
src/worker.rs      Job / Event / Op / Transfer 与 USB 工作线程
src/theme.rs       Tokens 与 egui Visuals
src/fonts.rs       内置中文字体子集
src/widgets.rs     页头、卡片/面板、字段行、墨色/描边/无底按钮、弹出字段、分段控件、圆点、徽章、表格、页脚、两栏布局
src/nav.rs         侧栏
src/device.rs      顶栏设备条、设备连接页
src/i2c/mod.rs     I2C 工作台（主机 / 从机页签）
src/i2c/master.rs  主机页、事务记录
src/i2c/slave.rs   从机页、256 B hex 网格
src/i2c/parse.rs   地址、字节、长度解析（规则同 CLI）
```
