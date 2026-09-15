#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-mode-router.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-mode-router.sh"

FUNDEPLOY_MODE_SERVICE="code-server"
FUNDEPLOY_MODE_OFFICIAL_SCRIPT="${SCRIPT_DIR}/setup-offical.sh"
FUNDEPLOY_MODE_MANUAL_SCRIPT="${SCRIPT_DIR}/setup-manual.sh"
FUNDEPLOY_MODE_OFFICIAL_DESC="官方 install.sh + 系统包管理器和系统服务"
FUNDEPLOY_MODE_MANUAL_DESC="官方 Release 包 + 本地 PID，默认安装到 ~/opt/code-server"
FUNDEPLOY_MODE_EXTRA_NOTE="官方模式额外支持 logs。"
FUNDEPLOY_MODE_MANUAL_NOTE="run 仅由 manual 支持。"
FUNDEPLOY_MODE_OFFICIAL_MENU="install.sh + 系统服务"
FUNDEPLOY_MODE_MANUAL_MENU="Release 包 + 本地 PID"

fundeploy_mode_router_main "$@"
