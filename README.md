# 密笺（Mijian）

AES-256-GCM、Argon2id、Ed25519 审计链、LWW-Map、Tokio TCP、egui。  
**绿色免安装**：单个 `mijian.exe` 即可运行，目标机无需安装 .NET / VC++ / 其它运行库。

**发布版本**：只改根目录 `Cargo.toml` 里 `[workspace.package] version`（当前 `0.3.0`）。界面显示、Windows 文件属性、更新检查、绿色版说明均自动跟从此处；Git 标签建议 `v` + 同号（如 `v0.3.0`）。

## 系统要求（重要）

| 平台 | 工具链 | 说明 |
|------|--------|------|
| **Windows 绿色包** | **Rust 1.77.2** + crt-static | 兼容 **Win7 SP1+ / 8 / 10 / 11 x64**；须用 `pack-green.bat` |
| Linux / macOS | **stable（较新）** | `rust-toolchain.toml` 默认 stable；打包用 `pack-green.sh` / `pack-green-macos.sh` |

若 Windows 包出现 **`ProcessPrng` / `bcryptprimitives.dll`**，说明用了新版 rustc，请改用 `pack-green.bat` 重新构建。

## 绿色包构建

| 平台 | 桌面（含 UI） | 无 UI 服务器 |
|------|--------------|--------------|
| Windows（1.77.2 + 静态 CRT） | `pack-green.bat` → `dist\mijian.exe` | `pack-server.bat` → `dist\mijian-server.exe`（兼 `mijian.exe`） |
| Linux（stable ≥ 1.78） | `./pack-green.sh` | `./pack-server.sh` |
| macOS | `./pack-green-macos.sh` | `./pack-server-macos.sh` |

```bat
pack-green.bat
pack-server.bat
```

```bash
chmod +x pack-green.sh pack-server.sh pack-green-macos.sh pack-server-macos.sh
./pack-green.sh          # 桌面
./pack-server.sh         # 服务器（--no-default-features）
```

| 构建 | feature | 依赖 |
|------|---------|------|
| 桌面（默认） | `ui` | GTK / X11 / OpenGL；Ubuntu 见下方 apt |
| 服务器 | 无 | 不需要桌面库 |

Ubuntu 桌面编译：

```bash
sudo apt install build-essential pkg-config libgl1-mesa-dev \
  libgtk-3-dev libgdk-pixbuf2.0-dev libayatana-appindicator3-dev libxdo-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
```

服务器不建终端用户：设 `MEMO_LOGIN_PASSWORD` 后 `mijian --no-ui` 会自动创建**存储节点身份**。详见 [docs/server-node.md](docs/server-node.md)。

运维（打包发布、数据目录、检查更新域名、密钥注意）：[docs/ops.md](docs/ops.md)。

依赖版本为兼容 Win7 构建做了钉死（同一 `Cargo.lock`）；Linux/macOS 仍用新 rustc 编译这些依赖即可。  
渲染使用 **OpenGL(glow)**。

## 设置路径

| 平台 | 路径 |
|------|------|
| Windows | `%AppData%\Mijian\settings.json` |
| macOS | `~/Library/Application Support/Mijian/settings.json` |
| Linux | `~/.config/Mijian/settings.json` |

首次启动若发现旧版 `%AppData%\DistributedMemo\`，会自动迁到 `Mijian`。

## 运行

```bash
mijian.exe              # GUI（无黑框）
mijian --no-ui --identity <指纹>   # 无窗口主节点（服务器）
```

Linux 无 `DISPLAY`/`WAYLAND_DISPLAY` 时自动走无 UI，并默认主机角色。详见 [docs/server-node.md](docs/server-node.md)。

### 双机联调（局域网自动发现）

1. 两机「设置」里 **`cluster_salt_hex` 相同**（与主密码无关；主密码仅本机解锁，各机可不同）。  
2. 开启 **局域网自动发现**（默认开）：同网段会通过 **UDP 17000** 广播互相发现，右侧「节点」列表显示发现/在线状态，无需手填对端。  
3. 跨网段或无广播环境：在「对端列表」手动填对方 `IP:TCP端口`，保存后重启。  
4. 防火墙放行：**TCP 监听端口**（默认 7000）以及 **UDP 17000**。

Headless 可用 `discover` 查看发现列表，`peers` 查看已连接。
