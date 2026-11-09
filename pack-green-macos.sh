#!/usr/bin/env bash
# macOS 对应 pack-green.bat：发布构建并复制到 dist/
set -euo pipefail
cd "$(dirname "$0")"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "请在 macOS 上运行本脚本（当前: $(uname -s)）" >&2
  exit 1
fi

ARCH="$(uname -m)"
echo "[1/2] 发布构建（memo-cli --release，宿主机架构: ${ARCH}，toolchain=$(rustc -V)）..."
# 勿随意 cargo update：会拉到 edition2024 依赖，与 rust-toolchain 1.77.2 不兼容
cargo build -p memo-cli --release --locked

mkdir -p dist
cp -f target/release/memo dist/memo
chmod +x dist/memo

echo
echo "完成: dist/memo"
echo "说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000"
echo "无窗口常驻: ./memo --no-ui --identity <指纹>（详见 docs/server-node.md / deploy/launchd）"
echo "架构: ${ARCH}（Apple Silicon 与 Intel 需分别在对应机器上构建，或自行配置交叉编译）"
echo
# 可选 notarize / codesign（售卖分发建议）:
#   codesign --force --sign "Developer ID Application: ..." dist/memo
echo
ls -lh dist/memo
