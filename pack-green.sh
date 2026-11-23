#!/usr/bin/env bash
# Linux 发布构建
# 桌面（默认，含 UI+托盘）: ./pack-green.sh
# 游戏/无图形服务器（不链 eframe/GTK）: MEMO_SERVER=1 ./pack-green.sh
set -euo pipefail
cd "$(dirname "$0")"

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"

MIN_RUST="1.78.0"
RUSTC_VER="$(rustc -V | awk '{print $2}')"
printf '%s\n' "$MIN_RUST" "$RUSTC_VER" | sort -V | head -n1 | grep -qx "$MIN_RUST" || {
  echo "需要 rustc >= $MIN_RUST（当前: $RUSTC_VER）。请: rustup update stable" >&2
  exit 1
}

# 请用 MEMO_SERVER=1。旧名 MEMO_NO_TRAY / MEMO_NO_UI 仍映射到服务器构建（已弃用）。
if [[ "${MEMO_NO_TRAY:-0}" == "1" || "${MEMO_NO_UI:-0}" == "1" ]]; then
  echo "提示: 请改用 MEMO_SERVER=1（MEMO_NO_TRAY/MEMO_NO_UI 已弃用）" >&2
  MEMO_SERVER=1
fi
SERVER="${MEMO_SERVER:-0}"

CARGO_ARGS=(build -p memo-cli --release --locked)
OUT_NAME="memo"

if [[ "$SERVER" == "1" ]]; then
  CARGO_ARGS+=(--no-default-features)
  OUT_NAME="memo-server"
  echo "模式: 服务器（无 UI，不依赖 GTK/X11/OpenGL）"
else
  echo "模式: 桌面（UI + 托盘）"
  if command -v pkg-config >/dev/null 2>&1; then
    missing=()
    pkg-config --exists gdk-pixbuf-2.0 || missing+=("libgdk-pixbuf2.0-dev")
    pkg-config --exists gtk+-3.0 || missing+=("libgtk-3-dev")
    if ! pkg-config --exists ayatana-appindicator3-0.1 \
      && ! pkg-config --exists appindicator3-0.1; then
      missing+=("libayatana-appindicator3-dev")
    fi
    pkg-config --exists xcb || missing+=("libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev")
    pkg-config --exists xkbcommon || missing+=("libxkbcommon-dev")
    if ! pkg-config --exists libxdo 2>/dev/null && ! ldconfig -p 2>/dev/null | grep -q 'libxdo\.so'; then
      missing+=("libxdo-dev")
    fi
    if ((${#missing[@]})); then
      echo "缺少系统开发包，Ubuntu/Debian 请安装:" >&2
      echo "  sudo apt install build-essential pkg-config libgl1-mesa-dev ${missing[*]}" >&2
      echo "或服务器无 UI 构建: MEMO_SERVER=1 ./pack-green.sh" >&2
      exit 1
    fi
  fi
fi

echo "[1/2] 发布构建（${CARGO_ARGS[*]}，toolchain=$(rustc -V)）..."
cargo "${CARGO_ARGS[@]}"

mkdir -p dist
cp -f target/release/memo "dist/${OUT_NAME}"
chmod +x "dist/${OUT_NAME}"
# 服务器包同时提供 memo 名，便于 systemd ExecStart
if [[ "$SERVER" == "1" ]]; then
  cp -f "dist/${OUT_NAME}" dist/memo
  chmod +x dist/memo
fi

echo
echo "完成: dist/${OUT_NAME}"
if [[ "$SERVER" == "1" ]]; then
  echo "首次: export MEMO_LOGIN_PASSWORD=... ; ./dist/memo --no-ui   # 自动建存储节点（非用户账号）"
  echo "常驻: 同目录再启即可；可选 --identity <指纹>"
else
  echo "桌面直接运行；无窗口主节点仍可用 --no-ui"
fi
echo
ls -lh dist/memo*
