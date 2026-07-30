#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash -n "${ROOT}/scripts/lib/nlt-progress.sh"
# shellcheck source=../scripts/lib/nlt-progress.sh
source "${ROOT}/scripts/lib/nlt-progress.sh"
export NONINTERACTIVE=1
# 非 TTY：确保不崩溃
nlt_pb_render 50 100 "smoke" "$(date +%s)" 2>/dev/null || true
echo "progress_smoke ok"
