# 密笺运维指南

面向打包发布、配置与日常运维。终端用户说明见应用内「使用说明」；无窗口存储主节点细节见 [server-node.md](server-node.md)。

## 1. 产品与组件

| 组件 | 说明 |
|------|------|
| 桌面绿色版 | 单文件 `mijian.exe`（或各平台等价产物），含 GUI |
| 无 UI 服务器 | `pack-server.*` 产物，`--no-ui` / 无显示器时作存储主节点 |
| 配置根 | 存放 `settings.json`、可选 `license.json`（与数据目录可分离） |
| 数据根 `data_dir` | 身份、审计、托管密文等业务数据 |

版本号唯一来源：仓库根目录 `Cargo.toml` 的 `[workspace.package] version`。界面显示、Windows 文件属性、更新检查、绿色版说明均跟从此处。

## 2. 配置与数据目录

### 配置文件位置

| 平台 | 路径 |
|------|------|
| Windows | `%AppData%\Mijian\settings.json` |
| macOS | `~/Library/Application Support/Mijian/settings.json` |
| Linux | `~/.config/Mijian/settings.json` |

首次启动若发现旧版 `%AppData%\DistributedMemo\`，会自动迁到 `Mijian`；`settings` 里仍指向旧路径的 `data_dir` 也会尽量纠正。

### 数据目录 `data_dir`

- **默认（Windows 选盘）**：`{盘符}\MemoData\{node_id}`
- **自定义**：新建/导入身份时可「自定义…」选任意文件夹，该路径即数据根
- 也可在 `settings.json` 中直接改 `"data_dir"`（改后需能访问；无 UI 节点务必显式配置）

### 数据根布局（勿擅自改名对调）

```
{data_dir}/
  schema.json                 # 当前为 schema v2
  identities/{指纹}/          # 身份元数据、密封私钥 secret.key
  vaults/{指纹}/              # 该身份业务数据
    audit.jsonl               # 备忘真源（哈希链）
    keys/                     # 审计签名密钥
    persons.json.enc 等
    hosted/                   # 私密托管密文（主节点侧）
```

**注意：** `vaults/{指纹}` 与 `identities/{指纹}` 必须对应同一身份私钥。对调 vault 文件夹名不会让账号互解，通常导致解密失败或开库失败。

## 3. 桌面打包发布

| 平台 | 桌面（含 UI） | 无 UI 服务器 |
|------|--------------|--------------|
| Windows | `pack-green.bat` → `dist\mijian.exe` | `pack-server.bat` |
| Linux | `./pack-green.sh` | `./pack-server.sh` |
| macOS | `./pack-green-macos.sh` | `./pack-server-macos.sh` |

**Windows 绿色包**须用 **Rust 1.77.2** + `pack-green.bat`（静态 CRT），兼容 Win7 SP1+。若出现 `ProcessPrng` / `bcryptprimitives.dll`，说明用了过新的 rustc，请按 README「系统要求」重打。

### 应用图标

| 用途 | 路径 |
|------|------|
| 登录页 / 托盘 / 窗口 | `crates/mijian-app/assets/app_icon.png` |
| exe 资源图标 | `crates/mijian-cli/assets/app_icon.ico` |

更换源图后覆盖上述文件，再执行 `pack-green.bat`（或 `cargo build -p mijian-cli --release`）。`build.rs` 会监视 `.ico` 变化。

## 4. 检查更新

客户端**只检查并提示**，打开下载页，**不自动安装**。

### 修改发布域名

编辑 [`crates/mijian-app/src/update_check.rs`](../crates/mijian-app/src/update_check.rs)：

```rust
pub const VERSION_JSON_URL: &str = "https://你的域名/.../latest.json";
pub const DOWNLOAD_PAGE_URL: &str = "https://你的域名/.../download";
```

改完后重新打包桌面版。

### `latest.json` 约定

```json
{
  "version": "0.3.0",
  "download_url": "https://你的域名/下载页或安装包地址"
}
```

- `version`：必填，与 workspace 版本比较  
- `download_url`：可选；缺省则打开 `DOWNLOAD_PAGE_URL`

当前仓库默认指向本机 commerce-hub 示例地址（`127.0.0.1:8080`），上线前务必改成正式域名。

## 5. 身份与密钥运维

| 材料 | 作用 |
|------|------|
| `.memokey` | 导出的身份包：含 Ed25519 **私钥明文**（新版）、指纹、别名、签名。等同根密钥，须保密保管 |
| 登录密码 | 仅密封本机 `identities/{指纹}/secret.key`，**不是**备忘正文密钥，也**不**用于托管加密 |
| `content_key` | `SHA256("memo-content-v1" ‖ 身份私钥)`，**不落盘**；解锁时在内存计算 |

私有备忘备份到其他主节点时，用**本身份 `content_key`** 加密后推送；主节点只存密文，不可解正文。恢复需要同一把身份私钥。

## 6. 无窗口主节点（摘要）

- 构建：`pack-server.bat` / `./pack-server.sh`（`--no-default-features`）
- 运行：`mijian --no-ui`；首次可用 `MEMO_LOGIN_PASSWORD` 自动建存储节点身份
- 防火墙：TCP `listen_port`（默认 7000）、UDP `17000`
- systemd / launchd：见 `deploy/systemd/`、`deploy/launchd/`

完整步骤、环境变量与角色说明：[server-node.md](server-node.md)。

## 7. 常见问题

| 现象 | 排查 |
|------|------|
| 检查更新失败 / 一直连本机 | 是否仍使用 `update_check.rs` 默认 `127.0.0.1`；发布域名与 `latest.json` 是否可达 |
| 旧版节点打不开、提示旧数据 | 启动时会尝试 PEM→v2 升级；失败可备份后按界面引导清空或换新 `data_dir` |
| 换盘/换目录后列表空 | 确认 `settings.json` 的 `data_dir` 指向实际有 `identities/` 的目录 |
| Win7 无法运行绿色包 | 是否用 1.77.2 + `pack-green.bat` 构建 |
| 备份到主节点后主节点看不到明文 | 预期行为：托管密文仅所有者可解 |
| 对调了 `vaults` 下文件夹名 | 停止该操作；用备份恢复正确对应关系 |
