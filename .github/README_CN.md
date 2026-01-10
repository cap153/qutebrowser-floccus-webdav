# qb-floccus

[![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](#LICENSE)
[English](README.md)  

**qb-floccus** 是一个专为 **Qutebrowser** 编写的高性能双向书签同步工具。

它通过 **WebDAV 协议** 同步标准的 **XBEL (XML Bookmark Exchange Language)** 文件，打通了 Qutebrowser（纯文本书签）与其他浏览器（通过 Floccus 插件）之间的数据壁垒，实现跨电脑的书签层级同步。

---

## ⚠️ 风险警告与免责声明 (必读)

**使用本软件即代表您知晓并接受：本软件涉及对您本地文件及远程云端数据的【覆盖】、【写入】和【删除】操作。**

**本软件按“原样”提供，作者不提供任何形式的明示或暗示担保（包括但不限于适销性、特定用途适用性）。在任何情况下，对于因使用或无法使用本软件而导致的任何数据丢失、文件损坏、业务中断或任何其他商业损害或损失，作者概不负责，即使作者已被告知发生此类损害的可能性。**

尽管我们在开发中引入了快照机制、熔断保护和状态码校验等安全措施，但**软件缺陷、网络异常或配置错误仍可能导致数据丢失**。

在使用本软件之前，您必须知晓并接受以下风险：

1.  **同步逻辑风险**：本工具支持“双向删除”。如果您在某一段错误地清空了书签且快照机制未能拦截，该“清空”操作可能会被同步到另一端。
2.  **网络传输风险**：在上传/下载过程中若发生极端网络波动，可能导致 XML 文件截断或损坏。
3.  **配置错误风险**：错误的 WebDAV URL 或权限配置可能导致程序误判文件状态。
4.  **并发冲突**：若多台设备在两次同步间隔内同时修改了同一书签（例如分别移动到了不同文件夹），本工具采用 **“本地优先 (Local Wins)”** 策略，将强制以 **运行本工具的 Qutebrowser 本地状态** 为准，覆盖远程端的修改。

**🛡️ 用户责任：**

为了您的数据安全，请务必遵守以下**用户责任**：

1.  **备份责任**：您有责任定期备份您的本地书签 (`~/.config/qutebrowser/bookmarks/urls`) 和 WebDAV 端的 XML 文件 (`bookmarks.xbel`)。**不要完全依赖本工具的自动化机制。**
2.  **格式确认**：请确保您的 Floccus 插件在“文件格式”中选择了 **XBEL**（默认值）。**本工具不支持 HTML 格式同步，强制运行可能导致文件内容被错误覆盖。**
3.  **自检责任**：首次运行前，请务必使用 `--check-dupe` 参数检查数据状况。

---

## ✅ v0.2.x 安全性改进

在 v0.2.x 版本中，我们重构了核心逻辑以降低上述风险：
*   **DOM 增量修改**：弃用全量重写模式。**未修改的书签节点（包括重复项）在 WebDAV 端会原封不动地保留。**
*   **元数据保护**：完美保留 Floccus 生成的 UUID、创建时间、父节点 ID 等元数据，不破坏 XML 结构。
*   **熔断机制**：内置 404/401 错误熔断，防止配置错误导致死循环。

---

## ✨ 特性

*   **⚡ 极速同步**：基于 Rust 编写，资源占用极低。
*   **🛡️ 无损编辑**：采用 DOM 解析引擎，**只修改变化的书签节点**，完美保留 Floccus 的 ID 及自定义元数据，不破坏原有 XML 结构。
*   **🔄 三方合并**：引入快照 (Snapshot) 机制，支持智能的“移动”判定，精准识别并同步“新增、删除、移动、重命名”操作。
*   **📂 层级映射**：独创使用 ` 📂 ` 分隔符，将 WebDAV 的文件夹结构映射为 Qutebrowser 的扁平标题，保留完整的目录层级上下文。
*   **🔒 安全配置**：支持通过 Shell 命令动态获取凭证（环境变量、Pass、GPG 等），配置文件不存明文。
*   **🤖 守护进程**：内置定时任务（Daemon Mode）与错误熔断机制。

### 💡 关于排序与移动
*   **移动**：支持将书签在不同文件夹间移动，变化会自动同步。
*   **排序**：本工具**完全忽略**并**不同步**自定义排序。
    *   **Qutebrowser 端**：强制按 `文件夹 > 标题` 字母顺序排序（这是 Qutebrowser 的本地文件特性）。
    *   **WebDAV 端 (Floccus)**：**同文件夹内的排序是自由的。** 您可以在 Chrome/Firefox 中随意调整书签顺序，该顺序**不会**被同步到 Qutebrowser，也**不会**被本工具重置（除非您对该书签进行了移动操作）。
    *   **新增书签**：从 Qutebrowser 同步到 WebDAV 的新书签，默认追加到目标文件夹的**末尾**。

---

## 📦 安装

### 方式一：源码编译 & 自动部署

如果您安装了 [Rust 工具链](https://rustup.rs/)，Makefile 会自动完成编译、安装以及 **Systemd 服务的生成与注册**。

```bash
git clone https://github.com/cap153/qb-floccus.git
cd qb-floccus

# 编译并安装到 ~/.local/bin，同时自动配置 Systemd
make install

systemctl --user daemon-reload
# 启动服务并配置开机自启动
systemctl --user enable --now qb-floccus
```

### 方式二：下载二进制文件 (手动部署)

1.  从 [Releases](https://github.com/cap153/qb-floccus/releases) 下载二进制文件并放入 PATH（如 `~/.local/bin/`）。
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
# WebDAV 服务器地址
# 程序会自动识别，如果 URL 不以 .xbel 结尾，会自动追加 /bookmarks.xbel
url = "http://192.168.1.x:8080"

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

**关于重复项的处理策略 (v0.2.x)：**
*   **Qutebrowser 本地**：若远程存在重复 URL，本地仅读取 XML 中最后出现的那一条。
*   **WebDAV 远程**：**程序仅执行只读检查，不会主动合并或删除远程的重复项。** 除非您在本地明确删除了该书签，否则远程的重复项将一直保留，互不影响。

建议首次运行前执行查重，了解数据状况：

```bash
qb-floccus --check-dupe
```

该命令仅读取不修改，会输出详细的重复项报告。

## 🖥️ Windows 支持说明

本项目代码逻辑在设计上完全兼容 Windows（包括路径处理、命令执行），但**尚未在 Windows 环境下进行完整测试**。

## 🙏 致谢

*   感谢 [Floccus](https://github.com/floccusaddon/floccus) 项目实现跨浏览器同步。
*   本项目是一个独立的实现，与 Floccus 使用的数据格式兼容，但不隶属于其开发人员。

## 🪪 License

[MIT](../LICENSE)
