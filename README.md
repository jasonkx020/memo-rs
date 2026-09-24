# 分布式安全备忘录（Rust）

AES-256-GCM、Argon2id、Ed25519 审计链、LWW-Map、Tokio TCP、egui。  
**绿色免安装**：单个 `memo.exe` 即可运行，目标机无需安装 .NET / VC++ / 其它运行库。

当前版本：**0.2.0**

## 授权说明

| 版本 | 行为 |
|------|------|
| 社区版（无 `license.json`） | 仅本机加密存储，**禁止**局域网发现/同步 |
| 商业版（有效 `license.json`） | 解锁 UDP 发现 + TCP 同步 |

将签发的 `license.json` 放到配置目录（与 `settings.json` 同级）后重启：

| 平台 | 配置目录 |
|------|----------|
| Windows | `%AppData%\DistributedMemo\` |
| macOS | `~/Library/Application Support/DistributedMemo/` |
| Linux | `~/.config/DistributedMemo/` |

签发工具（需发行方私钥，勿提交仓库）：

```bash
# 私钥：环境变量 MEMO_LICENSE_PRIVATE_HEX，或仓库根目录 license-private.hex（已 gitignore）
cargo run -p license-sign -- --licensee "客户名" --out license.json
```

## 系统要求（重要）

用 **Rust 1.77.2 + crt-static** 打出的绿色包可在 **Windows 7 SP1 / 8 / 10 / 11（64 位）** 运行。  
若出现 **`ProcessPrng` / `bcryptprimitives.dll`**，说明拿到的是用更新版 Rust 编的 exe，请改用本仓库 `pack-green.bat` / `dist\memo.exe`。

## 绿色包构建（Windows）

```bat
pack-green.bat
```

或：

```bash
cargo build -p memo-cli --release
# 复制 target/release/memo.exe 即可分发
```

已配置 **静态链接 CRT**（`.cargo/config.toml`），一般不必再装 VC++ Redistributable。  
渲染使用 **OpenGL(glow)**，降低对新显卡驱动的依赖。

**Authenticode 签名（售卖分发建议）**：设置环境变量 `MEMO_SIGN_CERT` 为证书 SHA1 指纹后运行 `pack-green.bat`，脚本会调用 `signtool`（需本机已安装 Windows SDK）。未设置则跳过签名。

## 安全模型（简要）

- 正文：Argon2id 派生密钥 + AES-256-GCM；标题等元数据明文落盘并随同步传输。
- 同步：TCP 上基于完整 `salt` 的 HMAC Hello 握手（防仅嗅探 `salt_fp` 的伪节点）；**无 TLS**，面向信任局域网。
- 审计：Ed25519 签名链式 `audit.jsonl`。

## 设置路径

| 平台 | 路径 |
|------|------|
| Windows | `%AppData%\DistributedMemo\settings.json` |
| macOS | `~/Library/Application Support/DistributedMemo/settings.json` |
| Linux | `~/.config/DistributedMemo/settings.json` |

## 运行

```bash
memo.exe              # GUI（无黑框）
memo.exe --headless   # 控制台守护
```

### 双机联调（局域网自动发现）

1. 两机均放置有效商业授权，使用 **同一主密码**，设置里 **`salt_hex` 相同**；节点 ID / TCP 端口可不同。  
2. 开启 **局域网自动发现**（默认开）：同网段会通过 **UDP 17000** 广播互相发现。  
3. 跨网段或无广播环境：在「对端列表」手动填对方 `IP:TCP端口`，保存后重启。  
4. 防火墙放行：**TCP 监听端口**（默认 7000）以及 **UDP 17000**。

Headless 可用 `discover` 查看发现列表，`peers` 查看已连接。

## 测试

```bash
cargo test --workspace
```

## 与 Go 版

对照工程：`../memo`。本版体积约数 MB，远小于 Fyne 版。
