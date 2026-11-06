# `uds-flow-manifest` v1 方案（据代码整理）

| 项目 | 内容 |
| --- | --- |
| 日期 | 2026-11-06 |
| 代码基线 | `1a33ecb`（PR #7 合并，2026-11-06） |
| 依据 | `host/crates/gui/src/flow/manifest.rs`、`fixtures/uds/*.udsflow.json`、PR #7 的描述 |
| 状态 | **不是原设计文档。** `manifest.rs` 引用的原文 `uds-flow-manifest-v1-方案.md` 没有找到；本文按代码和 PR #7 的描述整理，供评审和后续 PR 对照。与代码冲突时以代码为准 |
| 相关 | 真实刷写用的 v2 见[端到端实施计划第 7 节](uds-flash-implementation-plan.md) |

## 1. 定位

- `uds-flow-manifest` v1 是项目自拟的声明式流程清单，和 `.udsmock` 一样只用于**模拟**。它不是 ODX、PDX、OTX 或厂家格式；用它跑出的结果仍是模拟结果，不能作为真实刷写证据。
- 当前只合入了解析层（PR #7，即原设计 §8 中的 “PR A”）。只有测试会调用 `parse`；`MockBackend` 和界面还不读这个格式。
- 检查只读清单文本本身。`files` 里列出的文件只当作名字，从不打开；`sha256` 只检查格式，不和文件内容核对。

## 2. 本文内容的来源

| 内容 | 可信度 |
| --- | --- |
| 第 10 节 T1–T8 | PR #7 的描述逐条列出，按原意翻译 |
| 第 3、4、5 节的规则 | 从 `manifest.rs` 逐条整理，对应的测试见第 9 节 |
| 第 7 节示例 | PR #7 说明 3 个示例文件与原设计 §7.1–§7.3 逐字节相同；本文只引用文件，不重复正文 |
| 第 8 节分阶段 | 只知道 PR A 已合入，以及 PR #7 列出的“范围之外”；原设计 §8 的完整计划未知 |
| 小节编号 | 按代码注释（§3、§3.4、§5、§10）和 PR #7（§7、§8）对齐；其余小节的原始划分未知 |

## 3. 字段

所有对象都用 `#[serde(flatten)]` 收集多余的键，未知键一律报“不支持”（T7，见 5.3）。下表中“缺少”指检查结果“缺少依赖”（Missing），“不支持”指 Unsupported。

### 3.1 顶层

| 字段 | 类型 | 规则 |
| --- | --- | --- |
| `format` | 字符串 | 必须是 `"uds-flow-manifest"`，否则是硬错误 `NotRecognised` |
| `version` | 任意 JSON 值 | 必须是整数 `1`，否则不支持；没有时记“缺少 version”，显示为 `?` |
| `package` | 字符串，可选 | 只用于显示；为空时用调用方给的标签（文件名） |
| `notes` | 字符串数组，可选 | 只用于显示 |
| `files` | 数组 | 没有时记“缺少 files 清单” |
| `entries` | 数组 | 为空或没有时记“包内没有入口” |

### 3.2 `files[]`

| 字段 | 规则 |
| --- | --- |
| `path` | 必填；路径规则见 5.2；同一路径出现两次报“重复” |
| `sha256` | 可选（T3）；有就必须是 64 位小写十六进制；只检查格式，并在检查备注里写明“未与文件内容核对” |
| `role` | 可选；`image` / `driver` / `other` |
| `media` | 可选；`bin` / `hex` / `s19` / `unknown` |

### 3.3 `entries[]`

| 字段 | 规则 |
| --- | --- |
| `id` | 必填，非空白；包内唯一 |
| `title` | 可选；为空时用 `id` |
| `ecu` | 必填，非空白，否则记“未声明目标 ECU” |
| `images` | 见 3.4 |
| `needs` | 路径数组（例如 Flash Driver）；每项按 5.2 检查，并且必须出现在 `files` 中 |
| `params` | 见 3.5 |
| `steps` | 见 3.6；为空时记“没有可执行步骤（缺少 Flash Job）” |
| `simulate` | 见 3.7 |
| `requires` | 任意 JSON 数组；**每一项**都报“需要 …，模拟后端不提供” |

另外：有 `download` 步骤而没有任何有效镜像时，记“下载步骤没有镜像”。

### 3.4 `images[]`

| 字段 | 规则 |
| --- | --- |
| `file` | 必填；按 5.2 检查；必须出现在 `files` 中，镜像的格式取自该文件的 `media` |
| `base` | 可选；`"0x…"` 字符串，最多 32 位（5.4） |
| `len` | 可选；整数，大于 0，不超过 32 位 |

- 文件的 `media` 不是 `hex` / `s19`（包括 `bin`、`unknown`，以及文件不在 `files` 里）时，`base` 和 `len` 都必须给出。代码注释写的是“不替用户编造地址（§3.4）”。
- `base + len` 不能超过 4 GiB。

### 3.5 `params[]`

形状与后端 API 的 `api::Param` 一致：

| 类型 | 写法 |
| --- | --- |
| 可修改 | `{id, label, unit?, editable: true, lo, hi, default}`，三个数都是 32 位以内的整数，`lo ≤ default ≤ hi`；不能带 `value` |
| 只读 | `{id, label, unit?, editable: false, value}`，`value` 是字符串；不能带 `lo` / `hi` / `default` |

`id`、`label`、`editable` 必填；入口内 `id` 唯一。

### 3.6 `steps[]`

| 字段 | 规则 |
| --- | --- |
| `kind` | 必填，非空白；必须在白名单内（与 `.udsmock` 共用 `mock::KINDS`） |
| `label` | 必填，非空白 |
| `effect` | 可选，布尔值；省略时取白名单的默认值 |

白名单及 `effect` 默认值：

| kind | effect 默认 |
| --- | --- |
| `read_id`、`session`、`security`、`verify` | `false` |
| `erase`、`download`、`reset` | `true` |

- `erase` / `download` / `reset` 写成 `effect: false` 时报“不支持”，并仍按 `true` 处理。宿主“不重试有副作用的步骤”的规则依赖这个标志，所以清单不能把它关掉。
- 不在白名单里的 `kind` 报“不支持”，但仍保留在步骤列表里用于显示。
- 步骤只有展示级信息，没有 SID、DID、Routine ID 之类的参数。

### 3.7 `simulate`

- 只能写 `failAt`（该步骤以负响应结束）或 `lostAt`（后端在该步骤中途失联）其中一个，值是本入口中已有的步骤 `kind`。
- 调用方的 `Mode` 不是 `Simulated` 时，每个 `simulate` 都产生一条包级“不支持”（T8）。含 `simulate` 的包会标记 `simulated_only`。

## 4. 检查结果的分类

**硬错误（`ParseError`）只有四种**，表示根本不能当作清单读取：

| 错误 | 条件 |
| --- | --- |
| `TooLarge` | 超过 256 KiB（`host::MAX_ASSET`） |
| `Malformed` | 不是 JSON：语法错误、UTF-8 无效、被截断 |
| `NotRecognised` | 是 JSON，但没有 `"format": "uds-flow-manifest"` |
| `Shape` | 声明了 `format`，但某个值的 JSON 类型不对（例如 `"len": "262144"`），消息带行号和列号 |

其余问题都是检查结果（缺少 / 不支持），**任何一条都会阻止准备**。归属分两级：

- **包级**（阻止所有入口）：顶层未知键、`version`、`files[]` 的问题、没有入口、入口缺 `id`、入口 `id` 重复、T8 拒绝。
- **入口级**：其余各项。`Manifest::blockers(entry)` 先列包级，再列该入口的。

## 5. 限制

### 5.1 大小与编码

- 清单本身是裸 JSON，不打包（T2），上限 256 KiB。
- 开头的 UTF-8 BOM 会被去掉。

### 5.2 路径

`files[].path`、`images[].file`、`needs[]` 用同一套规则，下列情况都拒绝：

- 空字符串；
- 远程地址（含 `://`，或以 `http:` / `https:` 开头），不拉取；
- 含反斜杠、含冒号（包括盘符）；
- 以 `/` 开头的绝对路径；
- 含 `..` 段；
- 含空段或 `.` 段，或含控制字符。

### 5.3 未知键与禁止的键

任何层级的未知键都报“不支持”，不跳过（T7）。以下键另有说明：

| 键 | 处理 |
| --- | --- |
| `script` / `scripts` / `hooks` | 报“不支持”，并在检查备注里记“离线检查只记录，未执行” |
| `uri` / `url` | 报“远程地址，不拉取” |
| `frames` | 报“自由报文列表不受支持” |
| `eval` | 报“内嵌代码，不执行” |

**重复键**（2026-11-06 实测，serde 1.0.229 / serde_json 1.0.154，结构与 `RawManifest` 相同）：已知字段重复时是硬错误（`Shape`；`format` 重复且最后一个值不对时为 `NotRecognised`）；未知键重复时只保留最后一个值，但仍作为未知键报“不支持”。所以 v1 实际上不接受任何重复键。现有测试没有覆盖这一点。

### 5.4 十六进制值

地址写成 `"0x…"` 字符串，最多 32 位。裸数字（例如 `"65536"`）拒绝，因为分不清是十进制还是十六进制。

## 6. 与 `.udsmock` 和各后端的关系

- `.udsmock` 保留不变（T4）；两种格式共用同一份步骤白名单。
- 3 个示例清单与同名 `.udsmock` 等价：经 `MockBackend::inspect` 得到的入口 id、标题、ECU、步骤、参数、镜像和问题类型都相同（测试 `fixtures_match_their_udsmock_twins`）。
- `parse` 要求调用方传入自己的 `Mode`，用于执行 T8。
- `host_path_never_reaches_usb_or_processes` 测试也扫描 `manifest.rs`，保证解析层不碰 USB、不起进程。
- 真实刷写不使用 v1：自有后端只接受 v2，读到 v1 时报“v1 只有展示级步骤，不能真实执行”（端到端计划 7.1 节）。

## 7. 示例

| 原设计小节 | 文件 | 内容 |
| --- | --- | --- |
| §7.1 | `host/crates/gui/fixtures/uds/single.udsflow.json` | 单入口，7 种步骤各一个，无检查问题 |
| §7.2 | `host/crates/gui/fixtures/uds/multi.udsflow.json` | 多入口，必须显式选择；含 `failAt` / `lostAt` |
| §7.3 | `host/crates/gui/fixtures/uds/blocked.udsflow.json` | 缺少 Flash Driver、`requires`、`script`、不在白名单的步骤，准备被阻止 |

三个文件都没有写 `sha256`。§7.1–§7.3 与文件的对应关系按文件顺序推断。

## 8. 分阶段

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| PR A | 本文描述的解析层 | 已合入（PR #7，`1a33ecb`） |
| PR B | `MockBackend` 读取本格式；界面“来源”和文件选择入口；把 `Manifest` 映射为 `Inspection`；字体子集扫描加上 `.json` | 未开始（PR #7 列为范围之外） |
| 以后 | ZIP 打包、真实的 SHA-256 核对、HEX / S19 解析 | PR #7 列为范围之外，归属哪个阶段未知 |
| 原设计 §8 的其余内容 | 例如 “v1a” 之后的阶段 | 未知 |

和 v2 的关系：v2 把 `sha256` 改为必填并核对文件内容，也解析 HEX / S19，与上表“以后”一行有重叠。如果找回原设计，需要核对它的 §8 是否已经规划了 v1b 之类的阶段，再决定这些能力是放进 v1 的后续阶段，还是只在 v2 中提供。另外，T5 说的是清单本身不打包；v2 计划中 PDX 的 zip 限制（端到端计划 8.5 节）针对的是 PDX 容器，两者不冲突。

## 9. 现有测试

`manifest.rs` 中的 14 个测试（PR #7 合入时 `cargo test -p usb-i2c-gui --all-targets --locked` 共 160 个通过）：

| 测试 | 覆盖 |
| --- | --- |
| `fixtures_parse_as_documented` | 3 个示例的解析结果 |
| `fixtures_match_their_udsmock_twins` | 与 `.udsmock` 等价 |
| `every_fixture_file_is_a_manifest` | 示例目录中的每个 `*.udsflow.json` 都能解析 |
| `wrong_format_or_version_is_refused` | `format` / `version` 不对、JSON 损坏或被截断、类型错误 |
| `size_limit_and_bom` | 大小上限、BOM |
| `unknown_keys_are_refused_not_skipped` | 各层级的未知键（T7） |
| `scripts_urls_frames_and_eval_are_refused` | 5.3 的禁止键 |
| `paths_stay_inside_the_package` | 5.2 的路径规则 |
| `step_kinds_are_whitelisted` | 白名单和 `effect` |
| `simulate_needs_a_simulated_backend` | T8（`Mode::Hardware`） |
| `images_need_addresses_unless_segmented` | 3.4 |
| `params_follow_the_api_shape` | 3.5 |
| `files_and_entries_are_checked` | 3.2、3.3 |
| `errors_say_what_is_wrong` | 硬错误的提示文字 |

## 10. 锁定的决定（T1–T8）

依据 PR #7 描述中的 “Locked decisions (§10)” 表。

| # | 决定 | 代码中的落实 |
| --- | --- | --- |
| T1 | 格式标识 `uds-flow-manifest`，裸清单扩展名 `.udsflow.json` | `manifest::FORMAT`、`manifest::EXTENSION` |
| T2 | 只用裸 JSON，不打 ZIP 包 | `parse` 只读一个 JSON 文档，不依赖 zip |
| T3 | SHA-256，在 v1a 中可选 | `files[].sha256` 可选；有就必须是 64 位小写十六进制；只检查格式，不读文件内容 |
| T4 | 保留 `.udsmock` | `mock.rs` 的识别、示例和界面导入都不变 |
| T5 | 暂不设 ZIP 限制 | 不适用（没有 ZIP） |
| T6 | 不引入 `schemars` | 未引入 |
| T7 | 严格拒绝未知键 | 每个对象都收集未知键，每个都成为阻止准备的“不支持”，各层级都如此 |
| T8 | 非模拟后端必须拒绝 `simulate` | `parse(…, mode)`；模式不是 `Simulated` 时，每个 `simulate` 产生一条包级“不支持”；`Manifest::simulated_only` 标记这类包 |
