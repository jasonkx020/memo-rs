#!/usr/bin/env bash
# macOS 无 UI 存储节点包（等价于 MEMO_SERVER=1 ./pack-green-macos.sh）
set -euo pipefail
cd "$(dirname "$0")"
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "请在 macOS 上运行（当前: $(uname -s)）" >&2
  exit 1
fi
export MEMO_SERVER=1
exec ./pack-green-macos.sh
