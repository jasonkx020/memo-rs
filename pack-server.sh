#!/usr/bin/env bash
# Linux 无 UI 存储节点包（等价于 MEMO_SERVER=1 ./pack-green.sh）
set -euo pipefail
cd "$(dirname "$0")"
export MEMO_SERVER=1
exec ./pack-green.sh
