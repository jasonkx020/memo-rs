#!/usr/bin/env bash
# macOS 发布构建
# 桌面（默认）: ./pack-green-macos.sh
# 无 UI 服务器: MEMO_SERVER=1 ./pack-green-macos.sh
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

# 请用 MEMO_SERVER=1。旧名 MEMO_NO_TRAY / MEMO_NO_UI 仍映射（已弃用）。
if [[ "${MEMO_NO_TRAY:-0}" == "1" || "${MEMO_NO_UI:-0}" == "1" ]]; then
  echo "提示: 请改用 MEMO_SERVER=1（MEMO_NO_TRAY/MEMO_NO_UI 已弃用）" >&2
  MEMO_SERVER=1
fi
SERVER="${MEMO_SERVER:-0}"
ARCH="$(uname -m)"

CARGO_ARGS=(build -p mijian-cli --release --locked)
OUT_NAME="mijian"
if [[ "$SERVER" == "1" ]]; then
  CARGO_ARGS+=(--no-default-features)
  OUT_NAME="mijian-server"
  echo "模式: 服务器（无 UI）· ${ARCH}"
else
  echo "模式: 桌面（UI + 托盘）· ${ARCH}"
fi

echo "[1/2] 发布构建（${CARGO_ARGS[*]}，toolchain=$(rustc -V)）..."
cargo "${CARGO_ARGS[@]}"

mkdir -p dist
cp -f target/release/mijian "dist/${OUT_NAME}"
chmod +x "dist/${OUT_NAME}"
if [[ "$SERVER" == "1" ]]; then
  cp -f "dist/${OUT_NAME}" dist/mijian
  chmod +x dist/mijian
fi

APP_VER="$(awk '/^\[workspace\.package\]/{p=1;next} p&&/^version = /{gsub(/"/,"",$3); print $3; exit}' Cargo.toml)"
echo
echo "完成: dist/${OUT_NAME}  （版本 ${APP_VER}）"
echo "无窗口常驻: ./mijian --no-ui --identity <指纹>（详见 docs/server-node.md）"
echo "架构: ${ARCH}"
echo
ls -lh dist/mijian*
