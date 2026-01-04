# qb-floccus

[![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](#LICENSE)
[English](README.md)  

**qb-floccus** 是一个专为 **Qutebrowser** 编写的高性能双向书签同步工具。

它通过 **WebDAV 协议** 同步标准的 **XBEL (XML Bookmark Exchange Language)** 文件，打通了 Qutebrowser（纯文本书签）与其他浏览器（通过 Floccus 插件）之间的数据壁垒，实现跨电脑的书签层级与顺序同步。

---

## ⚠️ 风险警告与免责声明 (必读)

**本软件涉及对您本地文件及远程云端数据的 覆盖、写入 和 删除 操作。**

尽管我们在开发中引入了快照机制、熔断保护和状态码校验等安全措施，但**软件缺陷、网络异常或配置错误仍可能导致数据丢失**。

在使用本软件之前，您必须知晓并接受以下风险：

1.  **同步逻辑风险**：本工具支持“双向删除”。如果您在某一段错误地清空了书签且快照机制未能拦截，该“清空”操作可能会被同步到另一端。
2.  **网络传输风险**：在上传/下载过程中若发生极端网络波动，可能导致 XML 文件截断或损坏（虽然 WebDAV PUT 通常是原子的，但依赖于服务端实现）。
3.  **配置错误风险**：错误的 WebDAV URL 或权限配置可能导致程序误判文件状态，从而触发错误的同步逻辑。
4.  **并发冲突**：若多台设备同时修改同一书签，本工具采用“Last Write Wins”策略，较旧的修改将被覆盖。

**🛡️ 用户责任：**
*   **格式确认**：请确保您的 Floccus 插件在“文件格式”中选择了 **XBEL**（这是默认值），本工具不支持 HTML 格式同步。
*   **备份**：请务必定期备份您的 `~/.config/qutebrowser/bookmarks/urls` 和 WebDAV 端的 `bookmarks.xbel`。
*   **自检**：首次运行前，请务必使用 `--check-dupe` 检查数据状况。

**本软件按“原样”提供，不附带任何明示或暗示的保证。作者不对因使用本软件导致的任何数据丢失、损坏或业务中断承担责任。**

---

## ✨ 特性

*   **⚡ 极速同步**：基于 Rust 编写，资源占用极低。
*   **🛡️ 三方合并**：引入快照 (Snapshot) 机制，通过对比 `本地 vs 远程 vs 快照`，精准识别并同步“删除”操作。
*   **📂 层级映射**：独创使用 ` 📂 ` 分隔符，将 WebDAV 的文件夹结构映射为 Qutebrowser 的扁平标题，支持模糊搜索。
*   **🔒 安全配置**：支持通过 Shell 命令动态获取凭证（环境变量、Pass、GPG 等），配置文件不存明文。
*   **🤖 守护进程**：内置定时任务（Daemon Mode），无需依赖外部 Crontab。

## 📦 安装

### 方式一：源码编译 & 自动部署 (推荐)

如果您安装了 Rust 工具链，Makefile 会自动完成编译、安装以及 **Systemd 服务的生成与注册**。

```bash
git clone https://github.com/yourname/qb-floccus.git
cd qb-floccus

# 编译并安装到 ~/.local/bin，同时自动配置 Systemd
make install

systemctl --user daemon-reload
# 启动服务并配置开机自启动
systemctl --user enable --now qb-floccus
```

### 方式二：下载二进制文件 (手动部署)

1.  从 [Releases](https://github.com/yourname/qb-floccus/releases) 下载二进制文件并放入 PATH（如 `~/.local/bin/`）。
2.  **手动配置 Systemd**（如下）：

创建文件 `~/.config/systemd/user/qb-floccus.service`：

```ini
[Unit]
Description=Qutebrowser Floccus Sync Daemon
After=network-online.target

[Service]
Type=simple
# 请根据实际安装位置修改路径
ExecStart=%h/.local/bin/qb-floccus
Restart=on-failure
RestartSec=60s

# 环境变量示例 (若配置文件中使用环境变量获取账号密码)
# Environment="WEBDAV_USER=myuser"

[Install]
WantedBy=default.target
```

启用服务并配置开机自启动：

```bash
systemctl --user daemon-reload
systemctl --user enable --now qb-floccus
```

---

## ⚙️ 配置指南

配置文件路径：
*   **Linux**: `~/.config/qutebrowser/qb-floccus.toml`
*   **Windows**: `%APPDATA%\qutebrowser\qb-floccus.toml`

### 配置文件模板

```toml
# 可以指定qutebrowser本地书签文件的路径，windows系统可以这样写
# local_path = 'C:\Users\Captain\AppData\Roaming\qutebrowser\config\bookmarks\urls'

# 同步间隔 (秒)，注释此行则为单次运行模式
interval = 900

[webdav]
# 直接写你的webdav服务器地址，例如 http://192.168.1.x:8080 也是可以的
url = "http://192.168.1.x:8080/bookmarks.xbel"
# --- 凭证获取 (推荐) ---
# 支持执行任意 Shell 命令，stdout 即为凭证，下面给出3种示例方法
# 方式 1: 读取环境变量 (可在 Systemd 中配置 Environment)
username_cmd = "echo $WEBDAV_USER"
# 方式 2: 读取加密存储 (如 pass)
password_cmd = "pass show webdav/sync"
# 方式 3: 读取本地文件
# password_cmd = "cat ~/.secret_pass"
# --- 凭证获取 (明文/不推荐) ---
# username = "admin"
# password = "123"
```

---

## 🚀 首次运行建议

由于 Qutebrowser 本地书签通常无重复，而 WebDAV 端常因多端同步产生重复 URL。
**同步时，本工具会自动保留文件末尾（最新）的那一条重复项。**

建议首次运行前执行查重，避免误删：

```bash
qb-floccus --check-dupe
```

该命令仅读取不修改，会输出详细的重复项报告。

## 🖥️ Windows 支持说明

本项目代码逻辑在设计上完全兼容 Windows（包括路径处理、命令执行），但**尚未在 Windows 环境下进行完整测试**。

## 致谢

*   感谢 [Floccus](https://github.com/floccusaddon/floccus) 项目实现跨浏览器同步。
*   本项目是一个独立的实现，与 Floccus 使用的数据格式兼容，但不隶属于其开发人员。

## License

[MIT](../LICENSE)
