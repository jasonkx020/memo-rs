#!/usr/bin/env bash
# macOS 发布构建（使用较新 Rust / rust-toolchain.toml 的 stable，非 Windows 1.77.2）
set -euo pipefail
cd "$(dirname "$0")"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "请在 macOS 上运行本脚本（当前: $(uname -s)）" >&2
  exit 1
fi

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"

MIN_RUST="1.78.0"
RUSTC_VER="$(rustc -V | awk '{print $2}')"
printf '%s\n' "$MIN_RUST" "$RUSTC_VER" | sort -V | head -n1 | grep -qx "$MIN_RUST" || {
  echo "需要 rustc >= $MIN_RUST（当前: $RUSTC_VER）。请: rustup update stable" >&2
  exit 1
}

ARCH="$(uname -m)"
echo "[1/2] 发布构建（memo-cli --release，宿主机架构: ${ARCH}，toolchain=$(rustc -V)）..."
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
