# qb-floccus

[![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](#LICENSE)
[中文文档](README_CN.md)

**qb-floccus** is a high-performance, bidirectional bookmark synchronization tool written specifically for **Qutebrowser**.

It uses the **WebDAV protocol** to sync standard **XBEL (XML Bookmark Exchange Language)** files, bridging the data gap between Qutebrowser (plain text bookmarks) and other browsers (via the Floccus extension), enabling cross-computer synchronization of bookmark hierarchy and order.

---

## ⚠️ Risk Warning & Disclaimer (MUST READ)

**This software involves OVERWRITING, WRITING, and DELETING your local files and remote cloud data.**

Although we have introduced mechanisms such as snapshots, circuit breakers, and status code validation during development, **software bugs, network anomalies, or configuration errors may still result in data loss.**

Before using this software, you must understand and accept the following risks:

1.  **Sync Logic Risks**: This tool supports "bidirectional deletion". If you mistakenly clear bookmarks on one end and the snapshot mechanism fails to intercept it, this "clearing" operation may be synced to the other end.
2.  **Network Transmission Risks**: Extreme network fluctuations during upload/download may cause XML file truncation or corruption (although WebDAV PUT is usually atomic, it depends on the server implementation).
3.  **Configuration Error Risks**: Incorrect WebDAV URLs or permission settings may cause the program to misjudge the file status, triggering incorrect sync logic.
4.  **Concurrency Conflicts**: If multiple devices modify the same bookmark simultaneously, this tool adopts a "Last Write Wins" strategy, meaning older changes will be overwritten.

**🛡️ User Responsibilities:**
*   **Format Confirmation**: Please ensure your Floccus plugin is set to use **XBEL** format (this is the default). HTML format is not supported.
*   **Backup**: You must periodically backup your local `~/.config/qutebrowser/bookmarks/urls` and the remote `bookmarks.xbel`.
*   **Self-Check**: Please run `--check-dupe` before the first run to inspect data status.

**THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED. THE AUTHOR IS NOT LIABLE FOR ANY DATA LOSS, DAMAGES, OR BUSINESS INTERRUPTION RESULTING FROM THE USE OF THIS SOFTWARE.**

---

## ✨ Features

*   **⚡ Blazing Fast Sync**: Written in Rust with a minimal footprint. Supports bidirectional addition, deletion, and modification.
*   **🛡️ 3-Way Merge**: Introduces a **Snapshot** mechanism to compare `Local vs Remote vs Snapshot`, accurately identifying and syncing "deletions" instead of simply overwriting.
*   **📂 Hierarchy Mapping**: Uses a unique ` 📂 ` separator to map the WebDAV folder structure to Qutebrowser's flat titles, maintaining support for fuzzy search.
*   **🔒 Secure Configuration**: Supports retrieving credentials dynamically via Shell commands (Environment variables, Pass, GPG, etc.). No plaintext passwords in config files.
*   **🤖 Daemon Mode**: Built-in timer for scheduled syncing, eliminating the need for external Crontab or Systemd Timers.

## 📦 Installation

### Method 1: Build from Source & Auto Deploy (Recommended)

Requires [Rust toolchain](https://rustup.rs/) installed. The Makefile will handle compilation, installation, and **Systemd service generation/registration**.

```bash
git clone https://github.com/yourname/qb-floccus.git
cd qb-floccus

# Compiles and installs to ~/.local/bin, and configures Systemd automatically
make install

systemctl --user daemon-reload
# Start service and enable auto-start on boot
systemctl --user enable --now qb-floccus
```

### Method 2: Download Binary (Manual Deployment)

1.  Download the binary from [Releases](https://github.com/cap153/qb-floccus/releases) and place it in your PATH (e.g., `~/.local/bin/`).
2.  **Manually configure Systemd** (as follows):

Create file `~/.config/systemd/user/qb-floccus.service`:

```ini
[Unit]
Description=Qutebrowser Floccus Sync Daemon
After=network-online.target

[Service]
Type=simple
# Please adjust the path according to your actual installation
ExecStart=%h/.local/bin/qb-floccus
Restart=on-failure
RestartSec=60s

# Environment variable example (if you use env vars for credentials in config)
# Environment="WEBDAV_USER=myuser"

[Install]
WantedBy=default.target
```

Enable the service and auto-start:

```bash
systemctl --user daemon-reload
systemctl --user enable --now qb-floccus
```

---

## ⚙️ Configuration Guide

Configuration file paths:
*   **Linux**: `~/.config/qutebrowser/qb-floccus.toml`
*   **Windows**: `%APPDATA%\qutebrowser\qb-floccus.toml`

### Configuration Template

```toml
# You can specify the path to the local Qutebrowser bookmarks file.
# Example for Windows:
# local_path = 'C:\Users\Captain\AppData\Roaming\qutebrowser\config\bookmarks\urls'

# Sync interval (seconds). Comment this line out for "Run-Once Mode".
interval = 900

[webdav]
# Direct WebDAV server address (e.g., http://192.168.1.x:8080 is also accepted)
# Must point to the bookmarks.xbel file eventually.
url = "http://192.168.1.x:8080/bookmarks.xbel"

# --- Credential Retrieval (Recommended) ---
# Supports executing any Shell command; stdout is used as the credential.
# Below are 3 examples:

# Method 1: Read environment variable (Can be set in Systemd Environment)
username_cmd = "echo $WEBDAV_USER"

# Method 2: Read from encrypted storage (e.g., pass)
password_cmd = "pass show webdav/sync"

# Method 3: Read from a local file
# password_cmd = "cat ~/.secret_pass"

# --- Credential Retrieval (Plaintext / Not Recommended) ---
# username = "admin"
# password = "123"
```

---

## 🚀 First Run Recommendation

Qutebrowser local bookmarks usually have no duplicates, but the WebDAV end often accumulates duplicate URLs due to multi-device syncing.
**During synchronization, this tool automatically keeps the entry at the end of the file (usually the newest one).**

It is recommended to run a duplicate check before the first sync to avoid accidental deletion of desired bookmarks:

```bash
qb-floccus --check-dupe
```

This command only reads data without modifying it and outputs a detailed duplicate report.

## 🖥️ Windows Support Note

The code logic of this project is designed to be fully compatible with Windows (including path handling and command execution), but **it has not been fully tested in a Windows environment**.

## Acknowledgments

*   Thanks to the [Floccus](https://github.com/floccusaddon/floccus) project for enabling cross-browser synchronization.
*   This project is an independent implementation compatible with the data format used by Floccus but is not affiliated with its developers.

## License

[MIT](../LICENSE)
