#!/usr/bin/env bash
# Linux 对应 pack-green.bat：发布构建并复制到 dist/
set -euo pipefail
cd "$(dirname "$0")"

echo "[1/2] 发布构建（memo-cli --release）..."
cargo build -p memo-cli --release

mkdir -p dist
cp -f target/release/memo dist/memo
chmod +x dist/memo

echo
echo "完成: dist/memo"
echo "说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000"
echo "无窗口服务器: ./memo --no-ui --identity <指纹>（详见 docs/server-node.md）"
echo
ls -lh dist/memo
