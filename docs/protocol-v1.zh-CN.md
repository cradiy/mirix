# Mirix 协议 v1

[English](protocol-v1.md) | 简体中文 · [基础使用](../README.zh-CN.md)

本协议定义 daemon IPC 与独立输入法进程的通信边界。
协议版本独立于程序、crate 和皮肤格式版本；不同构建的程序只要实现 v1 即可通信。
当前发送版本为 `1`，支持版本集合为 `[1]`。

## 帧格式

每个请求、响应及事件均使用相同封装，不是换行分隔 JSON：

| 偏移 | 长度 | 内容 |
| --- | --- | --- |
| 0 | 4 字节 | JSON 正文的 UTF-8 字节数，u32 little-endian |
| 4 | 2 字节 | 协议版本，u16 little-endian；v1 为 `01 00` |
| 6 | 指定长度 | UTF-8 JSON |

正文最大 `1,048,576` 字节；长度不包含 6 字节帧头。流可以分片或连续包含多帧。
接收方先检查长度及版本，再解析正文；阻塞读取在为正文分配内存、读取正文前完成检查。
长度超限或版本不支持时终止该通信，
不得尝试按当前消息格式解释正文。不支持的版本产生带收到版本和支持列表的
`UnsupportedVersion` 错误，不保证向不兼容对端发送业务层错误。

两个完整帧示例：

```text
Hello: 07 00 00 00 01 00 22 48 65 6c 6c 6f 22
Ping:  06 00 00 00 01 00 22 50 69 6e 67 22
```

Rust 使用 `encode_frame` / `decode_frame` 或
`write_message` / `read_message`；两者均检查版本。
`decode_frame` 对未收齐的受支持帧返回 `None`，成功时返回消息及已消费字节数。
SDK 导出 `PROTOCOL_VERSION`、`SUPPORTED_PROTOCOL_VERSIONS` 和 `FRAME_HEADER_SIZE`。

## 兼容规则

- v1 的消息名称、字段类型、必需性、枚举表示和行为语义是兼容边界，不能随程序升级任意修改。
- JSON 对象成员顺序不影响语义。未知对象字段可忽略，但不能假定对端认识新的枚举值或操作。
- 新增可选字段必须保证缺省行为与 v1 一致；不能依赖对端处理它才能正确工作。
- 破坏性变更需明确引入新协议版本，并按帧版本选择对应实现；不得在 v1 下偷换语义。
- 当前没有版本协商或其他版本的转换实现。不支持的版本明确拒绝，不以猜测格式兜底。
- 状态 `revision`、宿主 `session` 和候选 ID 用于防止过期操作，不是协议版本。

Rust SDK 的源码 API 不等于进程 ABI。非 Rust 引擎可实现相同线格式。
字段定义见 [引擎消息](../crates/mirix-engine-protocol/src/lib.rs)、
[输入状态](../crates/mirix-engine-protocol/src/engine.rs) 和
[daemon 消息](../crates/mirix-ipc/src/messages.rs)。

## 引擎通信

输入法安装目录名为 ID，入口固定为可执行文件 `engine`。宿主将工作目录设为安装目录，
并设置 `MIRIX_ENGINE_NAME`、`MIRIX_CONFIG_DIR`、`MIRIX_DATA_DIR`、`MIRIX_CACHE_DIR`。
后三者分别位于用户配置、数据和缓存根目录下的
`mirix/input-methods/<ID>`、`mirix/input-method-data/<ID>`、`mirix/input-methods/<ID>`；
引擎负责创建及维护自己的文件。扫描目录不执行引擎，启用后才启动。
Rust 实现可依赖 `mirix-engine-protocol`，实现 `Engine` 后调用 `serve`。

宿主通过子进程 stdin/stdout 交换帧；日志写 stderr。请求串行执行，
每个请求有一个普通响应，可在响应前收到 `IconChanged` 事件。
启动时宿主发送 JSON 字符串 `"Hello"`，引擎依次返回 `IconChanged`、`Ready`。
带字段消息使用外部标签，例如 `{"Select":{"id":42}}`。

| 请求 | 普通响应 |
| --- | --- |
| `Hello` | `Ready` |
| `ProcessKey`、`Select` | `State` |
| `Reset` | `ResetDone` |
| `SetCandidatePages` | `PagesSet` |
| `SelectMenu` | `MenuState` |
| `GetConfiguration`、`Configure` | `Configuration` |
| `Vocabulary` | `VocabularyState` |

普通操作失败返回 `Error { message }`，不表示传输损坏。
非法响应状态、超时和进程退出会使该引擎会话失效，不发布部分状态。
一次请求的写入及全部响应读取共用截止时间：握手 2 秒，输入操作 250 毫秒，
配置与词条操作 10 秒。引擎 SDK 的 `serve` 提供标准分发及发送前校验。

### 输入状态

`EngineOutput` 包含按键消费标记、可选提交、应用预编辑和面板预编辑更新。
候选及选中项与输出在同一响应内传递。可运行测试使用的
[完整 State 示例](../crates/mirix-engine-protocol/tests/fixtures/v1-state.json)
展示了应用字母输入、面板读音及候选的独立表示。

- `preedit`：`{"type":"keep"}`、`{"type":"clear"}` 或
  `{"type":"set","value":{"text":"ni","cursor":2,"selection":null}}`。
- `Composition.text` 非空；光标为 UTF-8 字节位置，选区为非空
  `{"start":0,"end":2}`，结束位置不包含在内。全部位置必须落在字符边界。
- 提交文字必须同时明确清空或替换预编辑，允许提交与下一段预编辑一起返回。
- `panel_composition` 为 `keep` 或 `set`；设置值为
  `follow_application`、`hidden` 或独立 `composition`。
  独立内容不受隐藏应用重复预编辑的规则影响，皮肤只负责样式。
- 失焦、重置和切换引擎清除两份预编辑；提交辅助函数恢复面板跟随模式。

`Candidate` 包含 `id`、`text`、可选 `comment`、可选 `reading` 和
`keep_open`。ID 是不透明 u64，同一快照内唯一；选择按 ID，不按下标。
`selected` 是零起始索引，空列表为 0。替换候选集合应生成新 ID，拒绝过期选择。
查询和无变化操作保留 ID、选中项及分页。

非空 `reading` 由引擎控制，优先于普通注释显示，不由框架猜测读音。
`keep_open: true` 表示等待选词结果时保持面板显示；最终状态仍由引擎响应决定。
键盘选词语义由引擎处理。

`SetCandidatePages.ends` 为递增的排他结束位置，最后一项等于候选数，
每页最多 9 项，空列表使用 `[]`。如 `[3,8,12]` 表示三页。
daemon 验证上下文及候选 ID 快照后传给引擎；候选改变时旧页边界失效。

### 配置与扩展接口

配置状态变化时引擎递增 `configuration_revision`。配置事件携带所依据的 revision，
过期状态不能写入。配置、菜单和词条操作可返回同一个 `EngineOutput`，
独立决定是否更新输入；成功不等于必须改变候选或提交文字。

**设置**：`Configuration { title, description, groups }` 描述页面，
每组包含 `id, label, controls`。控件包含 `id, label, kind`，
可选 `description`（空）、`enabled`（true）和 `tray`（false）。
组 ID 在页面内唯一，控件 ID 跨组唯一，显示文案按请求的 locale 返回。

| kind.type | 状态 |
| --- | --- |
| `switch`、`checkable` | `value: bool` |
| `checklist` | `values: string[]`、`choices` |
| `select` | `value: string`、`choices` |
| `path` | `value: string`、`target: "file" 或 "directory"` |
| `text` | `value`、`submit_label`，可选 `placeholder`、`clear_on_submit`（false） |
| `action` | 无 |

choices 为有序的 `{ value, label }` 数组，值唯一，选中值必须存在。
事件为 `{"type":"change","id":"...","value":...}` 或 `{"type":"invoke","id":"..."}`。
引擎校验、保存并返回权威页面；成功可携带 message，失败返回 Error。
文本仅在 Enter 或按钮点击时提交，clear_on_submit 仅在成功后清空；
路径控件只传递路径，不代替引擎读取文件。控件定义见
[configuration.rs](../crates/mirix-engine-protocol/src/configuration.rs)。

**菜单**：`EngineMenu { id, label, shortcut, entries, active }`，
entries 包含 `{ id, label }`。菜单 ID、快捷键在引擎内唯一，条目 ID 在菜单内唯一，
active 可为空，否则必须存在。宿主只处理导航，按不透明 ID 发送 SelectMenu。
框架快捷键优先；旧弹窗会话的选择失效。字段见
[menu.rs](../crates/mirix-engine-protocol/src/menu.rs)。

**图标**：`EngineIcon.images` 提供 1–4 个不同尺寸的 RGBA8 图像，
宽高各 1–128，非预乘 alpha、无填充，总像素数据最多 64 KiB。
每次 Hello 的 Ready 响应前发送 IconChanged，之后仅在值变化时先于普通响应发送，null 表示清除；
不支持空闲推送或图标动画。字段见 [icon.rs](../crates/mirix-engine-protocol/src/icon.rs)。

**个人词条**：Vocabulary 支持 List、Add、Remove、Forget、Pin、Hide。
词条以文字与完整读音识别；profile 是当前场景检查条件，不负责切换场景。
查询和无变化操作保持输入，实际修改可返回输入更新。
字段见 [vocabulary.rs](../crates/mirix-engine-protocol/src/vocabulary.rs)。

## daemon IPC

Unix socket 默认为 `$XDG_RUNTIME_DIR/mirix/mirix.sock`，
无该环境变量时使用 `/tmp/mirix/mirix.sock`；`--socket` 可覆盖。
单连接保持请求与响应顺序；订阅连接还会收到完整状态快照。
异步管理操作先返回 `Pending`，随后在同一连接返回最终结果。

| 用途 | 主要请求 |
| --- | --- |
| 生命周期 | `Ping`、`Restart`、`Shutdown` |
| 输入上下文 | `CreateContext`、`DestroyContext`、`FocusIn`、`FocusOut`、`Key`、`Select`、`Reset`、`SetCaps` |
| 输入法管理 | `SubscribeInputMethods`、`RefreshInputMethods`、`SetInputMethodEnabled`、`ReorderInputMethods`、`RestartInputMethod`、`SetShortcut` |
| 引擎操作 | `ToggleInputMethod`、`SetInputMethod`、`GetEngineConfiguration`、`ConfigureEngine`、`SelectEngineMenu`、`Vocabulary` |
| 面板 | `SubscribePanel`、`SetCandidatePages`、`EnableQuickSwitch`、`ChooseQuickSwitch`、`SetPanelSkins`、`UiPreferenceChanged` |
| 应用许可 | `GetImeState`、`SetImeEnabled` |

管理请求中的 `request` 用于关联结果，`session` 与 `revision` 防止旧界面覆盖新状态。
面板订阅与选词响应使用单调递增的快照 revision；断连后丢弃旧状态并重新订阅。
修改失败不自动重试，避免重复提交或写入。

此 socket 是本地用户会话控制接口，不验证调用者是否拥有某个应用窗口。

应用先发送 `"GetImeState"` 获取当前 context_id，再以
`{"SetImeEnabled":{"id":7,"enabled":false}}` 暂停，true 释放本连接的暂停。
请求返回合并后的 ImeState；无焦点时为 `context_id: null, enabled: false`。
暂停仅针对当前焦点周期，必须保持连接；断连、失焦或销毁时自动释放。
多连接暂停需全部释放才恢复；失效 ID 操作报错。
首次暂停取消组合且不提交文字，暂停期间按键绕过引擎；恢复不还原已取消的输入。
应用负责自己的焦点判断，不应伪造 FocusIn 控制其他窗口。
