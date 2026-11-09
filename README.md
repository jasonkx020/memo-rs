# 分布式安全备忘录（Rust）

AES-256-GCM、Argon2id、Ed25519 审计链、LWW-Map、Tokio TCP、egui。  
**绿色免安装**：单个 `memo.exe` 即可运行，目标机无需安装 .NET / VC++ / 其它运行库。

## 系统要求（重要）

用 **Rust 1.77.2 + crt-static** 打出的绿色包可在 **Windows 7 SP1 / 8 / 10 / 11（64 位）** 运行。  
若出现 **`ProcessPrng` / `bcryptprimitives.dll`**，说明拿到的是用更新版 Rust 编的 exe，请改用本仓库 `pack-green.bat` / `dist\memo.exe`。

## 绿色包构建

**Windows**（Rust 1.77.2 + 静态 CRT，见 `.cargo/config.toml`）：

```bat
pack-green.bat
```

产物：`dist\memo.exe`。目标机一般不必再装 VC++ Redistributable。

**Linux**：

```bash
chmod +x pack-green.sh   # 首次
./pack-green.sh
```

**macOS**：

```bash
chmod +x pack-green-macos.sh   # 首次
./pack-green-macos.sh
```

产物均为 `dist/memo`。GUI 需本机 OpenGL；无窗口主节点见 [docs/server-node.md](docs/server-node.md)。  
Apple Silicon 与 Intel 需分别在对应机器构建（或自行交叉编译）。

或手动：`cargo build -p memo-cli --release`。  
渲染使用 **OpenGL(glow)**，降低对新显卡驱动的依赖。

## 设置路径

| 平台 | 路径 |
|------|------|
| Windows | `%AppData%\DistributedMemo\settings.json` |
| macOS | `~/Library/Application Support/DistributedMemo/settings.json` |
| Linux | `~/.config/DistributedMemo/settings.json` |

## 运行

```bash
memo.exe              # GUI（无黑框）
memo --no-ui --identity <指纹>   # 无窗口主节点（服务器）
```

Linux 无 `DISPLAY`/`WAYLAND_DISPLAY` 时自动走无 UI，并默认主机角色。详见 [docs/server-node.md](docs/server-node.md)。

### 双机联调（局域网自动发现）

1. 两机「设置」里 **`cluster_salt_hex` 相同**（与主密码无关；主密码仅本机解锁，各机可不同）。  
2. 开启 **局域网自动发现**（默认开）：同网段会通过 **UDP 17000** 广播互相发现，右侧「节点」列表显示发现/在线状态，无需手填对端。  
3. 跨网段或无广播环境：在「对端列表」手动填对方 `IP:TCP端口`，保存后重启。  
4. 防火墙放行：**TCP 监听端口**（默认 7000）以及 **UDP 17000**。

Headless 可用 `discover` 查看发现列表，`peers` 查看已连接。

## 与 Go 版

对照工程：`../memo`。本版体积约数 MB，远小于 Fyne 版。
