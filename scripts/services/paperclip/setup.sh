#!/usr/bin/env bash
# Paperclip (https://github.com/paperclipai/paperclip) 本机服务。
# CLI 安装、升级和后台服务均交给 Paperclip 官方命令管理。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

PAPERCLIP_HOME="${PAPERCLIP_HOME:-${HOME}/.paperclip}"
PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID:-default}"
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
PAPERCLIP_NODE_MIN_VERSION="${PAPERCLIP_NODE_MIN_VERSION:-24.11.0}"
PAPERCLIP_NODE_MIN_MAJOR="${PAPERCLIP_NODE_MIN_MAJOR:-24}"
PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC="${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC:-60}"
PAPERCLIP_CONFIG_PATH="${PAPERCLIP_HOME}/instances/${PAPERCLIP_INSTANCE_ID}/config.json"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

命令:
  install          官方 managed install（正式版）
  install-canary   官方 managed install（开发版）
  install-prod     install 的别名
  update [canary|prod]  官方升级、备份、迁移及服务重启（默认 canary）
  onboard          首次配置；NONINTERACTIVE=1 时默认追加 --yes
  plugin list                  列出 awesome-paperclip 收录的插件
  plugin install [npm-package] 安装插件；不指定包名时从清单选择
  plugin installed             列出当前已安装的插件
  start             启动官方后台服务
  run               使用 paperclipai run 前台启动
  stop              停止官方后台服务
  restart           热重启官方后台服务
  status            查看官方 supervisor 与健康状态
  logs [options]    查看官方服务日志，例如 logs -f
  uninstall         卸载官方服务和 CLI，保留 ${PAPERCLIP_HOME} 数据

说明:
  - Linux 使用 systemd --user，macOS 使用 LaunchAgent。
  - 配置由 paperclipai onboard / configure 管理。
  - PAPERCLIP_INSTANCE_ID 默认为 ${PAPERCLIP_INSTANCE_ID}。
  - update 完成后检查 PostgreSQL、Paperclip 端口及 /api/health。
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

node_version_meets() {
  local min="$1"
  command -v node >/dev/null 2>&1 || return 1
  node -e '
    const min = process.argv[1].split(".").map(Number);
    const cur = process.versions.node.split(".").map(Number);
    for (let i = 0; i < 3; i++) {
      if ((cur[i] || 0) > (min[i] || 0)) process.exit(0);
      if ((cur[i] || 0) < (min[i] || 0)) process.exit(1);
    }
  ' "$min" 2>/dev/null
}

paperclip_ensure_node_version() {
  node_version_meets "${PAPERCLIP_NODE_MIN_VERSION}" && return 0

  local nvm_dir="${NVM_DIR:-${HOME}/.nvm}"
  [[ -s "${nvm_dir}/nvm.sh" ]] || return 0
  # shellcheck source=/dev/null
  source "${nvm_dir}/nvm.sh" >/dev/null 2>&1 || return 0
  command -v nvm >/dev/null 2>&1 || return 0
  nvm use "${PAPERCLIP_NODE_MIN_MAJOR}" >/dev/null 2>&1 || true
}

require_node() {
  paperclip_ensure_node_version
  command -v node >/dev/null 2>&1 || die "需要 Node.js ${PAPERCLIP_NODE_MIN_VERSION}+（https://nodejs.org/）"
  node_version_meets "${PAPERCLIP_NODE_MIN_VERSION}" || die "需要 Node.js ${PAPERCLIP_NODE_MIN_VERSION}+，当前: $(node --version)。可执行: nvm install ${PAPERCLIP_NODE_MIN_MAJOR}"
}

paperclip_load_config() {
  PAPERCLIP_DATABASE_MODE=""
  PAPERCLIP_DATABASE_DIR="${PAPERCLIP_HOME}/instances/${PAPERCLIP_INSTANCE_ID}/db"
  PAPERCLIP_DATABASE_PORT=5432
  PAPERCLIP_SERVER_PORT=8804
  [[ -f "${PAPERCLIP_CONFIG_PATH}" ]] || return 0

  local values
  values="$(node -e '
    const fs = require("node:fs");
    const path = require("node:path");
    const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const db = config.database || {};
    const server = config.server || {};
    const dataDir = db.embeddedPostgresDataDir
      ? path.resolve(db.embeddedPostgresDataDir.replace(/^~(?=\/)/, process.env.HOME || ""))
      : process.argv[2];
    console.log([db.mode || "", dataDir, db.embeddedPostgresPort || 5432, server.port || 8804].join("\t"));
  ' "${PAPERCLIP_CONFIG_PATH}" "${PAPERCLIP_DATABASE_DIR}")" || die "无法读取 Paperclip 配置: ${PAPERCLIP_CONFIG_PATH}"
  IFS=$'\t' read -r PAPERCLIP_DATABASE_MODE PAPERCLIP_DATABASE_DIR PAPERCLIP_DATABASE_PORT PAPERCLIP_SERVER_PORT <<<"$values"
}

paperclip_executable() {
  if [[ -x "${HOME}/.local/bin/paperclipai" ]]; then
    printf '%s\n' "${HOME}/.local/bin/paperclipai"
  else
    command -v paperclipai 2>/dev/null || return 1
  fi
}

paperclip_cli() {
  require_node
  local executable
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  paperclip_load_config
  if [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" ]]; then
    (unset DATABASE_URL; PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" "$@")
  else
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" "$@"
  fi
}

cmd_install() {
  require_node
  command -v npx >/dev/null 2>&1 || die "未找到 npx（Node.js 自带）"
  PAPERCLIP_HOME="${PAPERCLIP_HOME}" npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" paperclipai@latest install --yes "$@"
  paperclip_cli --version
}

paperclip_port_open() {
  local port="$1"
  if command -v nc >/dev/null 2>&1; then
    nc -z 127.0.0.1 "$port" >/dev/null 2>&1
  else
    (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null
  fi
}

paperclip_stop_embedded_postgres() {
  local pid_file="${PAPERCLIP_DATABASE_DIR}/postmaster.pid"
  [[ -f "$pid_file" ]] || return 0

  local pid command_line i
  pid="$(sed -n '1{s/[[:space:]]//g;p;}' "$pid_file")"
  [[ "$pid" =~ ^[0-9]+$ ]] || die "无效的 PostgreSQL PID 文件: ${pid_file}"
  kill -0 "$pid" 2>/dev/null || return 0
  command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command_line" == *postgres* && "$command_line" == *"${PAPERCLIP_DATABASE_DIR}"* ]] || die "PID ${pid} 不是 ${PAPERCLIP_DATABASE_DIR} 的 PostgreSQL，拒绝停止"

  echo "==> 停止 embedded PostgreSQL（PID ${pid}）" >&2
  kill -TERM "$pid"
  for ((i = 0; i < 30; i++)); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 1
  done
  die "embedded PostgreSQL 在 30 秒内未停止，已中止升级"
}

paperclip_allow_embedded_postgres_build() {
  [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" ]] || return 0
  [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || return 0
  command -v pnpm >/dev/null 2>&1 || return 0

  local package="@embedded-postgres/darwin-arm64" package_path project_dir major key current merged
  package_path="$(pnpm list -g --depth=-1 --json 2>/dev/null | node -e '
    const fs = require("node:fs");
    const rows = JSON.parse(fs.readFileSync(0, "utf8"));
    const hit = rows.find((row) => row.dependencies?.paperclipai?.path);
    if (hit) process.stdout.write(hit.dependencies.paperclipai.path);
  ' 2>/dev/null || true)"
  [[ -n "$package_path" ]] || return 0
  project_dir="$(dirname "$(dirname "$package_path")")"
  major="$(pnpm --version | cut -d. -f1)"

  if ((major >= 11)); then
    key=allowBuilds
    current="$(pnpm --dir "$project_dir" config get "$key" --json 2>/dev/null || true)"
    merged="$(printf '%s' "$current" | node -e '
      const fs = require("node:fs");
      let value = {};
      try { value = JSON.parse(fs.readFileSync(0, "utf8")); } catch {}
      value[process.argv[1]] = true;
      process.stdout.write(JSON.stringify(value));
    ' "$package")"
  else
    key=onlyBuiltDependencies
    current="$(pnpm --dir "$project_dir" config get "$key" --json 2>/dev/null || true)"
    merged="$(printf '%s' "$current" | node -e '
      const fs = require("node:fs");
      let value = [];
      try { value = JSON.parse(fs.readFileSync(0, "utf8")); } catch {}
      if (!value.includes(process.argv[1])) value.push(process.argv[1]);
      process.stdout.write(JSON.stringify(value));
    ' "$package")"
  fi

  pnpm --dir "$project_dir" config set --location=project --json "$key" "$merged"
}

paperclip_hydrate_embedded_postgres() {
  [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" ]] || return 0
  [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || return 0

  local -a roots=()
  local root scripts script package_root
  [[ -d "${PAPERCLIP_HOME}/cli" ]] && roots+=("${PAPERCLIP_HOME}/cli")
  if command -v pnpm >/dev/null 2>&1; then
    root="$(pnpm root -g 2>/dev/null || true)"
    [[ -d "$root" ]] && roots+=("$root")
  fi
  if command -v npm >/dev/null 2>&1; then
    root="$(npm root -g 2>/dev/null || true)"
    [[ -d "$root" ]] && roots+=("$root")
  fi
  ((${#roots[@]} > 0)) || die "找不到 Paperclip 安装目录，无法修复 embedded PostgreSQL 软链接"

  scripts="$(find "${roots[@]}" -type f -path '*/@embedded-postgres/darwin-arm64/scripts/hydrate-symlinks.js' 2>/dev/null)"
  [[ -n "$scripts" ]] || die "找不到 @embedded-postgres/darwin-arm64/scripts/hydrate-symlinks.js"
  while IFS= read -r script; do
    package_root="${script%/scripts/hydrate-symlinks.js}"
    (cd "$package_root" && node scripts/hydrate-symlinks.js)
    node -e '
      const fs = require("node:fs");
      const path = require("node:path");
      const root = process.argv[1];
      const rows = JSON.parse(fs.readFileSync(path.join(root, "native/pg-symlinks.json"), "utf8"));
      if (!rows.length || rows.some(({target}) => !fs.lstatSync(path.join(root, target)).isSymbolicLink())) process.exit(1);
    ' "$package_root" || die "embedded PostgreSQL 动态库软链接修复失败: ${package_root}"
  done <<<"$scripts"
}

paperclip_remove_launchd_database_url() {
  [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" && "$(uname -s)" == "Darwin" ]] || return 0
  local label="ing.paperclip.paperclipai.${PAPERCLIP_INSTANCE_ID}"
  [[ "${PAPERCLIP_INSTANCE_ID}" != "default" ]] || label="ing.paperclip.paperclipai"
  local plist="${HOME}/Library/LaunchAgents/${label}.plist"
  [[ -f "$plist" ]] || return 0
  if /usr/libexec/PlistBuddy -c 'Print :EnvironmentVariables:DATABASE_URL' "$plist" >/dev/null 2>&1; then
    /usr/libexec/PlistBuddy -c 'Delete :EnvironmentVariables:DATABASE_URL' "$plist" || die "无法从 ${plist} 删除 DATABASE_URL"
  fi
}

paperclip_wait_for_update_health() {
  paperclip_load_config
  local deadline=$((SECONDS + PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC)) targets="Paperclip ${PAPERCLIP_SERVER_PORT} 和 /api/health"
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || targets="PostgreSQL ${PAPERCLIP_DATABASE_PORT}、${targets}"
  echo "==> 检查 ${targets}" >&2
  while ((SECONDS <= deadline)); do
    if { [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || paperclip_port_open "${PAPERCLIP_DATABASE_PORT}"; } &&
      paperclip_port_open "${PAPERCLIP_SERVER_PORT}" &&
      curl --fail --silent --show-error --max-time 3 "http://127.0.0.1:${PAPERCLIP_SERVER_PORT}/api/health" >/dev/null; then
      echo "Paperclip 升级完成，健康检查通过。"
      return 0
    fi
    sleep 1
  done
  die "Paperclip 升级后未在 ${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC} 秒内通过健康检查"
}

cmd_update() {
  local channel="${1:-canary}" option
  case "$channel" in
    canary) option=--canary ;;
    prod) option=--latest ;;
    *) die "未知更新渠道: ${channel}（支持 canary / prod）" ;;
  esac

  paperclip_load_config
  [[ ! -f "${PAPERCLIP_CONFIG_PATH}" ]] || paperclip_cli db:backup

  if ! paperclip_cli service stop; then
    paperclip_port_open "${PAPERCLIP_SERVER_PORT}" && die "Paperclip 仍在监听 ${PAPERCLIP_SERVER_PORT}，中止升级"
  fi
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || paperclip_stop_embedded_postgres
  paperclip_allow_embedded_postgres_build

  if ! paperclip_cli update "$option" --no-backup; then
    paperclip_cli service start || true
    return 1
  fi
  paperclip_hydrate_embedded_postgres
  paperclip_remove_launchd_database_url
  paperclip_cli service start
  paperclip_wait_for_update_health
}

cmd_onboard() {
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    paperclip_cli onboard --yes "$@"
  else
    paperclip_cli onboard "$@"
  fi
}

paperclip_plugin_catalog_records() {
  cat <<'CATALOG'
Agent Pixels|@agent-pixels/paperclip-plugin|https://github.com/gcampton/Agent-Pixels|Pixel Agents for Paperclip with custom behaviors, models, rooms, and security cam access.
obsidian-paperclip||https://github.com/istib/obsidian-paperclip|Obsidian integration for browsing, commenting on, and assigning Paperclip issues.
paperclip-aperture|@tomismeta/paperclip-aperture|https://github.com/tomismeta/paperclip-aperture|Alternative Focus view that ranks approvals, issue activity, and human-facing events.
paperclip-live-analytics-plugin|@agent-analytics/paperclip-live-analytics-plugin|https://github.com/Agent-Analytics/paperclip-live-analytics-plugin|Live visitor map, dashboard widget, and Agent Analytics settings page.
paperclip-plugin-acp|paperclip-plugin-acp|https://github.com/mvanhorn/paperclip-plugin-acp|ACP runtime for Claude Code, Codex, and Gemini CLI from chat platforms.
paperclip-plugin-avp|paperclip-plugin-avp|https://github.com/creatorrmode-lead/paperclip-plugin-avp|Trust and reputation layer using Agent Veil Protocol.
paperclip-plugin-chat|@paperclipai/plugin-chat|https://github.com/webprismdevin/paperclip-plugin-chat|Interactive AI chat copilot for tasks, agents, and workspaces.
paperclip-plugin-company-wizard|@yesterday-ai/paperclip-plugin-company-wizard|https://github.com/yesterday-ai/paperclip-plugin-company-wizard|AI-powered company setup assistant with presets.
paperclip-plugin-discord|paperclip-plugin-discord|https://github.com/mvanhorn/paperclip-plugin-discord|Bidirectional Discord integration.
paperclip-plugin-github-issues|paperclip-plugin-github-issues|https://github.com/mvanhorn/paperclip-plugin-github-issues|Bidirectional GitHub Issues sync.
paperclip-plugin-linear|@oldharlem/paperclip-plugin-linear|https://github.com/Oldharlem/paperclip-linear-plugin|Bidirectional Linear sync with webhooks and an agent tool.
paperclip-plugin-slack|paperclip-plugin-slack|https://github.com/mvanhorn/paperclip-plugin-slack|Slack notifications for issue lifecycle events.
paperclip-plugin-telegram|paperclip-plugin-telegram|https://github.com/mvanhorn/paperclip-plugin-telegram|Telegram notifications for issue lifecycle events.
paperclip-plugin-writbase|paperclip-plugin-writbase|https://github.com/Writbase/paperclip-plugin-writbase|Bidirectional sync between Paperclip issues and WritBase tasks.
paperclip-plugin-hindsight|paperclip-plugin-hindsight|https://github.com/vectorize-io/hindsight/tree/main/hindsight-integrations/paperclip-plugin|Persistent long-term memory for Paperclip agents.
CATALOG
}

cmd_plugin_list() {
  local name package url description
  trap '' PIPE
  while IFS='|' read -r name package url description; do
    printf '%s%s\n  %s\n  %s\n' "$name" "${package:+ (${package})}" "$description" "$url" 2>/dev/null || return 0
  done < <(paperclip_plugin_catalog_records)
}

cmd_plugin_install() {
  local package="${1:-}" pick name url description i
  if [[ -n "$package" ]]; then
    shift
    paperclip_cli plugin install "$package" "$@"
    return
  fi

  local -a labels=() packages=() urls=()
  while IFS='|' read -r name package url description; do
    [[ -n "$package" ]] || continue
    labels+=("${name} - ${description}")
    packages+=("${package}")
    urls+=("${url}")
  done < <(paperclip_plugin_catalog_records)

  pick="$(fundeploy_ui_choose "Paperclip / 从 awesome-paperclip 选择插件" "${labels[@]}")" || return 0
  for i in "${!labels[@]}"; do
    [[ "${labels[$i]}" == "$pick" ]] || continue
    echo "==> 安装 Paperclip 插件 ${packages[$i]}（来源: ${urls[$i]}）" >&2
    paperclip_cli plugin install "${packages[$i]}"
    return
  done
  die "无法识别所选插件"
}

cmd_plugin() {
  local action="${1:-install}"
  [[ $# -eq 0 ]] || shift
  case "$action" in
    list) cmd_plugin_list ;;
    install) cmd_plugin_install "$@" ;;
    installed) paperclip_cli plugin list "$@" ;;
    *) die "未知插件命令: ${action}（支持 list / install / installed）" ;;
  esac
}

cmd_run() {
  require_node
  local executable
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  paperclip_load_config
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || unset DATABASE_URL
  exec env PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" run "$@"
}

cmd_uninstall() {
  echo "Paperclip 数据目录 ${PAPERCLIP_HOME} 不会删除。" >&2
  fundeploy_confirm_destructive "确认卸载 Paperclip 官方服务和 CLI？" PAPERCLIP_UNINSTALL_YES || return 1

  paperclip_cli service uninstall
  paperclip_cli uninstall
  echo "已卸载 Paperclip 服务和 CLI；数据保留在 ${PAPERCLIP_HOME}。"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install | install-prod) cmd_install ;;
    install-canary) cmd_install --canary ;;
    update) cmd_update "$@" ;;
    onboard) cmd_onboard "$@" ;;
    plugin) cmd_plugin "$@" ;;
    run) cmd_run "$@" ;;
    start | stop | restart | status | logs) paperclip_cli service "$cmd" "$@" ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *) echo "未知命令: ${cmd}" >&2; usage >&2; exit 2 ;;
  esac
}

interactive_main() {
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  fundeploy_ui_banner "Paperclip 官方服务" "PAPERCLIP_HOME=${PAPERCLIP_HOME}" "instance=${PAPERCLIP_INSTANCE_ID}"
  echo ""
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / paperclip / 选择动作" \
      "install-canary" "install-prod" "update" "onboard" "plugin install" "start" "run" "stop" "restart" "status" "logs" "uninstall" "help" "quit")" || break
    [[ -n "$pick" ]] || break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      "plugin install") ( dispatch plugin install ) ;;
      *) ( dispatch "$pick" ) ;;
    esac
    echo ""
  done
  set -e
}

main() {
  if [[ $# -eq 0 ]]; then
    _fundeploy_ensure_gum || exit 1
    interactive_main
  else
    dispatch "$@"
  fi
}

main "$@"
