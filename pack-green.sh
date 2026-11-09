#!/usr/bin/env bash
# Linux 发布构建（使用较新 Rust / rust-toolchain.toml 的 stable，非 Windows 1.77.2）
set -euo pipefail
cd "$(dirname "$0")"

# 不沿用 Win7 的 1.77.2；若环境误装了旧 override，优先用 stable
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"

MIN_RUST="1.78.0"
RUSTC_VER="$(rustc -V | awk '{print $2}')"
printf '%s\n' "$MIN_RUST" "$RUSTC_VER" | sort -V | head -n1 | grep -qx "$MIN_RUST" || {
  echo "需要 rustc >= $MIN_RUST（当前: $RUSTC_VER）。请: rustup update stable" >&2
  exit 1
}

echo "[1/2] 发布构建（memo-cli --release，toolchain=$(rustc -V)）..."
cargo build -p memo-cli --release --locked

mkdir -p dist
cp -f target/release/memo dist/memo
chmod +x dist/memo

echo
echo "完成: dist/memo"
echo "说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000"
echo "无窗口服务器: ./memo --no-ui --identity <指纹>（详见 docs/server-node.md）"
echo
ls -lh dist/memo
