#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-mode-router.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-mode-router.sh"

FUNDEPLOY_MODE_SERVICE="sub2api"
FUNDEPLOY_MODE_OFFICIAL_SCRIPT="${SCRIPT_DIR}/setup-offical.sh"
FUNDEPLOY_MODE_MANUAL_SCRIPT="${SCRIPT_DIR}/setup-manual.sh"
FUNDEPLOY_MODE_OFFICIAL_DESC="上游官方脚本 + systemd，默认安装到 /opt/sub2api"
FUNDEPLOY_MODE_MANUAL_DESC="本地二进制 + PID 管理，默认安装到 ~/opt/sub2api"
FUNDEPLOY_MODE_EXTRA_NOTE="官方模式额外支持 logs，默认端口均为 8802。"
FUNDEPLOY_MODE_MANUAL_NOTE="run 和版本查询仅由 manual 支持。"
FUNDEPLOY_MODE_OFFICIAL_MENU="上游脚本 + systemd"
FUNDEPLOY_MODE_MANUAL_MENU="本地二进制 + PID"
FUNDEPLOY_MODE_MANUAL_ACTIONS="list-versions versions"

fundeploy_mode_router_main "$@"
