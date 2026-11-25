# 无窗口服务器主节点

桌面 GUI 结构不变。在 **无图形环境**（或显式 `--no-ui`）下，`memo` 默认以 **主机（Master）** 常驻，受理从节点私人备份与还原。

## 编译：是否依赖 UI

由 **Cargo feature `ui`** 控制（默认开启）：

| 构建 | 命令 | 系统依赖 |
|------|------|----------|
| 桌面 | `./pack-green.sh` / `pack-green.bat` / `./pack-green-macos.sh` | GTK / X11 / OpenGL 等 |
| 游戏/无图形服务器 | `./pack-server.sh` / `pack-server.bat` / `./pack-server-macos.sh` | **无需** GTK/X11 |

等价手动：`cargo build -p mijian-cli --release --no-default-features`。

服务器包不链接 `mijian-app`（eframe/托盘），适合未装桌面组件的机器。

## 数据目录（重要）

`WorkingDirectory` **不会**自动成为数据目录。请在 `settings.json` 中设置，例如：

```json
{
  "data_dir": "/var/lib/memo",
  "listen_port": 7000,
  "lan_discovery": true,
  "node_visible": true,
  "accept_foreign_backup": true
}
```

Linux 默认大致为 `~/.config/Mijian/...`。身份与托管库落在 `data_dir` 下的 `identities/`、`vaults/{指纹}/`（含 `hosted/`），**勿改路径布局**。

## 何时运行时走 no-ui

| 平台 | 行为 |
|------|------|
| 未编入 `ui` | 始终 no-ui |
| Linux + 有 `ui` | 无 `DISPLAY` 且无 `WAYLAND_DISPLAY` → 自动 no-ui |
| macOS / Windows + 有 `ui` | 默认 GUI；服务器加 `--no-ui` 或 `MEMO_NO_UI=1` |

无 UI 且未传 `--role` → 本次进程 Master（默认不写回 settings，除非 `--persist-role`）。  
`--headless` 为隐藏别名，文档与脚本请主推 `--no-ui`。

## 首次启动（无 GUI）

服务器**不需要创建终端用户**。协议需要**节点机器身份**（也可用环境变量 `MEMO_IDENTITY`）。

有解锁密码时，无身份会**自动**生成存储节点（默认别名 `storage`）：

```bash
export MEMO_LOGIN_PASSWORD='节点解锁密码'
# 可选：settings 里已设好 data_dir
memo --no-ui
# 首次打印指纹后，可写入环境：
# export MEMO_IDENTITY=<指纹>
```

或显式：`memo --no-ui --init --alias storage`，或 `--identity-file` 导入 `.memokey`。

## 防火墙

- TCP：`listen_port`（默认 7000）
- UDP：`17000`

## systemd / launchd

见 `deploy/systemd/`、`deploy/launchd/`。

- 首次可省略 `--identity`（靠 `MEMO_LOGIN_PASSWORD` 自动建节点）。
- 之后在 `memo.env` 填写 `MEMO_IDENTITY`；CLI 会读取该变量（也可由单元文件展开为 `--identity`）。
