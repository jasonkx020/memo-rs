#!/usr/bin/env bash
# Linux 发布构建（stable；默认含 Ubuntu 桌面托盘）
set -euo pipefail
cd "$(dirname "$0")"

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"

MIN_RUST="1.78.0"
RUSTC_VER="$(rustc -V | awk '{print $2}')"
printf '%s\n' "$MIN_RUST" "$RUSTC_VER" | sort -V | head -n1 | grep -qx "$MIN_RUST" || {
  echo "需要 rustc >= $MIN_RUST（当前: $RUSTC_VER）。请: rustup update stable" >&2
  exit 1
}

# MEMO_NO_TRAY=1：无桌面服务器，不链 GTK/AppIndicator
NO_TRAY="${MEMO_NO_TRAY:-0}"
CARGO_FEATURES=(--locked)
if [[ "$NO_TRAY" == "1" ]]; then
  CARGO_FEATURES+=(--no-default-features)
  echo "模式: 无托盘（服务器）"
else
  echo "模式: 桌面托盘（Windows/macOS/Ubuntu 兼容）"
  if command -v pkg-config >/dev/null 2>&1; then
    missing=()
    pkg-config --exists gdk-pixbuf-2.0 || missing+=("libgdk-pixbuf2.0-dev")
    pkg-config --exists gtk+-3.0 || missing+=("libgtk-3-dev")
    # tray-icon → libappindicator（Ubuntu 常用 ayatana 兼容包）
    if ! pkg-config --exists ayatana-appindicator3-0.1 \
      && ! pkg-config --exists appindicator3-0.1; then
      missing+=("libayatana-appindicator3-dev")
    fi
    pkg-config --exists xcb || missing+=("libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev")
    pkg-config --exists xkbcommon || missing+=("libxkbcommon-dev")
    if ((${#missing[@]})); then
      echo "缺少系统开发包，Ubuntu/Debian 请安装:" >&2
      echo "  sudo apt install build-essential pkg-config libgl1-mesa-dev ${missing[*]}" >&2
      exit 1
    fi
  fi
fi

echo "[1/2] 发布构建（memo-cli --release，toolchain=$(rustc -V)）..."
cargo build -p memo-cli --release "${CARGO_FEATURES[@]}"

mkdir -p dist
cp -f target/release/memo dist/memo
chmod +x dist/memo

echo
echo "完成: dist/memo"
echo "说明: 复制该文件即可使用；同网段自动发现需放行 TCP 端口 + UDP 17000"
echo "无窗口服务器: ./memo --no-ui --identity <指纹>"
echo "无 GTK 编译: MEMO_NO_TRAY=1 ./pack-green.sh"
echo
ls -lh dist/memo
