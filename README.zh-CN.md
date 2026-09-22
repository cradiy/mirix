# Mirix

[English](README.md) | 简体中文

Linux 输入法框架，使用 Rust 和 [GPUI](https://github.com/cradiy/gpui)。
支持 Wayland 输入、XWayland/XIM、独立进程引擎、候选框皮肤和引擎定义的设置页面。
随项目提供英文键盘与拼音引擎。

Mirix 许可证：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。

[协议 v1](docs/protocol-v1.zh-CN.md) · [拼音使用说明](docs/pinyin.zh-CN.md)

## 为什么写 Mirix

我想要一个界面现代、皮肤可定制、引擎与界面解耦的 Linux 输入法框架，
让输入体验由自己掌控。

## 特点

- 现代化候选框皮肤：圆角、渐变、图片图层，以及可开关的动效。
- 横排与竖排候选、预编辑显示和读音注释。
- 引擎独立运行，通过统一协议提供候选、设置、快捷键和托盘菜单。
- 输入法和皮肤以目录形式安装，在设置中启用和选择。

## 候选框预览

Night

![Night 候选框](docs/images/candidate-night.png)

Midnight

![Midnight 候选框](docs/images/candidate-midnight.png)

Night Sakura

![Night Sakura 候选框](docs/images/candidate-night-sakura.png)

## 拼音使用前准备

附带的拼音引擎不是开箱即用的输入法。项目仅提供单字表，需要自行导入词组库。
将 UTF-8 TSV 词库放入 `~/.config/mirix/input-methods/pinyin/dictionaries/`，
再在拼音设置中点击「重新加载词库」。格式见[拼音使用说明](docs/pinyin.zh-CN.md#添加拼音词库)。

## 安装

安装 Rust、Nushell 及 GPUI 所需系统依赖后执行：

```sh
nu scripts/deploy.nu
```

脚本构建 release 程序，安装 daemon、面板、引擎、附带皮肤及应用菜单入口。
已有配置和词库不覆盖。请用普通桌面用户执行，不要 `sudo nu`；受保护目录会单独请求 sudo。

- `--dry-run`：只查看安装路径，不修改文件。
- `--no-restart`：安装但不访问 systemd、不重启服务。
- `--update-skins`：备份并更新已安装的附带皮肤。

默认程序目录为 `/usr/local/bin`，已安装用户服务时会重启。
首次安装后，在 Wayland 桌面会话中退出其他输入法，再启用用户服务：

```sh
install -Dm644 deploy/mirix.service "$HOME/.config/systemd/user/mirix.service"
systemctl --user daemon-reload
systemctl --user enable --now mirix.service
```

也可直接运行 `mirixd --no-xim`，不要与服务同时启动。
合成器须支持 input-method、虚拟键盘和输入弹窗协议；应用需支持 Wayland 文本输入，
且未通过 `GTK_IM_MODULE` / `QT_IM_MODULE` 强制使用其他输入法。

```sh
systemctl --user restart mirix.service     # 重启
systemctl --user stop mirix.service        # 停止
journalctl --user -u mirix.service -f       # 查看日志
```

若服务缺少 Wayland 环境，在桌面会话终端运行
`systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP` 后重启服务。

## 使用

- 应用菜单中的 Mirix 启动用户服务，右键菜单可打开设置。
- `mirix-panel --settings` 管理输入法、切换顺序、快捷键、语言和皮肤。
- `Ctrl+Space` 按启用顺序循环切换输入法。
- 英文键盘支持 Compose、dead keys 和 `Ctrl+Shift+U` Unicode 输入。
- 托盘可选择输入法、皮肤及引擎提供的菜单。
- 输入方式、词库、场景和个人词条见 [拼音使用说明](docs/pinyin.zh-CN.md)。

## 配置与扩展

以下为默认路径，分别遵循 XDG_CONFIG_HOME 和 XDG_DATA_HOME：

| 路径 | 用途 |
| --- | --- |
| `~/.config/mirix/config.toml` | 启用列表、快捷键、语言和皮肤选择 |
| `~/.local/share/mirix/input-methods/<ID>/engine` | 输入法可执行文件 |
| `~/.local/share/mirix/skins/<ID>/skin.toml` | 皮肤包入口 |

安装输入法只需放入目录，再在设置中刷新并启用。`input_methods.enabled` 仅记录用户选择，
不是安装注册表。皮肤支持已安装的 ID 或具体路径，不扫描项目目录；安装后刷新皮肤列表。
界面修改自动保存；手动修改主配置后重启。显式命令行参数优先于保存值。

[mirix-engine-japanese](https://github.com/cradiy/mirix-engine-japanese.git) 是用于演示
Mirix 引擎接入的日文输入法示例，并非完善的输入法。

## 预览与开发

```sh
# 离线设置预览：不连接 daemon，不读写个人配置
cargo run -p mirix-panel -- --settings-demo examples/settings/workspace
cargo run -p mirix-panel -- --settings-demo examples/settings/controls.json

# 候选框预览
cargo run -p mirix-panel -- --demo --skin skins/midnight

# 集成测试需要先构建引擎可执行文件
cargo build --workspace --bins
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```
