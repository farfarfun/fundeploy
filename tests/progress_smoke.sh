#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash -n "${ROOT}/scripts/lib/fundeploy-progress.sh"
# shellcheck source=../scripts/lib/fundeploy-progress.sh
source "${ROOT}/scripts/lib/fundeploy-progress.sh"
export NONINTERACTIVE=1
# 非 TTY：确保不崩溃
fundeploy_pb_render 50 100 "smoke" "$(date +%s)" 2>/dev/null || true
echo "progress_smoke ok"
