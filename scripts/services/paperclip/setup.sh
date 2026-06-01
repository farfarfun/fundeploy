#!/usr/bin/env bash
# Paperclip（https://github.com/paperclipai/paperclip）本机服务：从 GitHub 克隆源码 + pnpm 安装 + 启停。
# 默认实例数据根 PAPERCLIP_HOME 与 PAPERCLIP_WORKSPACE 一致（~/opt/paperclip/workspace）；官方默认常为 ~/.paperclip，可用环境变量改回。
#
# 依赖：git、Node.js 20+、pnpm 9+（无 pnpm 时尝试 corepack enable）
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # 克隆/拉取源码并 pnpm install
#   ./setup.sh update       # git pull + pnpm install
#   ./setup.sh start        # 后台启动（pnpm paperclipai run）；无配置时会先非交互生成默认配置；默认轮询 /instance/scheduler-heartbeats 校验
#   ./setup.sh run          # 前台启动（同准备语义，终端附着；不写 PID；后台已在跑时拒绝）
#   ./setup.sh fix-embedded-postgres [版本]  # 显式安装 @embedded-postgres/<当前平台>（pnpm 常漏装 optional 平台包）；版本可降级
#   ./setup.sh onboard      # 上游首次配置（NONINTERACTIVE=1 时加 --yes）
#   ./setup.sh stop / restart / status
#
# 环境变量：
#   PAPERCLIP_SERVICE_HOME   本脚本管理根目录（默认 ~/opt/paperclip）
#   PAPERCLIP_REPO_URL       上游 Git（默认 https://github.com/paperclipai/paperclip.git）
#   PAPERCLIP_GIT_BRANCH     克隆分支（默认 main）
#   PAPERCLIP_PORT           监听与健康检查端口（默认 8804；启动时 export PORT 同值）
#   PAPERCLIP_WORKSPACE      本机默认工作区目录（默认 ${PAPERCLIP_SERVICE_HOME}/workspace，即 ~/opt/paperclip/workspace）
#   PAPERCLIP_HOME           上游数据根（默认与 PAPERCLIP_WORKSPACE 相同）；未设置时实例在 workspace 下，避免占用 ~/.paperclip
#   PAPERCLIP_INSTANCE_ID    实例 id（默认 default）
#   NONINTERACTIVE=1         跳过 gum 确认；onboard 子命令使用 --yes
#   PAPERCLIP_UNINSTALL_YES=1  非 TTY 卸载确认
#   PAPERCLIP_START_HEALTH_TIMEOUT_SEC  start 时等待 /instance/scheduler-heartbeats 返回 200 的最长时间秒数（默认 60）
#   PAPERCLIP_SKIP_START_HEALTH_CHECK=1   仅确认进程存活，不请求 HTTP（不推荐）
#   PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_VERSION  fix-embedded-postgres 默认使用的版本/SemVer 范围；命令行 [版本] 优先
#   PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_PKG   非标准平台时指定完整包名，如 @embedded-postgres/linux-x64-gnu
#   PAPERCLIP_EMBEDDED_POSTGRES_NO_REGISTRY_FALLBACK=1  fix-embedded-postgres 在 pnpm add 失败时不改用 registry latest 重试

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${SCRIPT_DIR}/../lib 与 ${SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

PAPERCLIP_SERVICE_HOME="${PAPERCLIP_SERVICE_HOME:-${HOME}/opt/paperclip}"
PAPERCLIP_REPO_URL="${PAPERCLIP_REPO_URL:-https://github.com/paperclipai/paperclip.git}"
PAPERCLIP_GIT_BRANCH="${PAPERCLIP_GIT_BRANCH:-main}"
PAPERCLIP_PORT="${PAPERCLIP_PORT:-8804}"

PAPERCLIP_SRC="${PAPERCLIP_SRC:-${PAPERCLIP_SERVICE_HOME}/src/paperclip}"
PAPERCLIP_WORKSPACE="${PAPERCLIP_WORKSPACE:-${PAPERCLIP_SERVICE_HOME}/workspace}"
PAPERCLIP_HOME="${PAPERCLIP_HOME:-${PAPERCLIP_WORKSPACE}}"
PAPERCLIP_RUN_DIR="${PAPERCLIP_SERVICE_HOME}/run"
PAPERCLIP_LOG_DIR="${PAPERCLIP_SERVICE_HOME}/log"
PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip.pid"
LOG_FILE="${PAPERCLIP_LOG_DIR}/paperclip.run.log"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

  无参数：gum 菜单。

命令:
  install     克隆 ${PAPERCLIP_REPO_URL} 到 ${PAPERCLIP_SRC}（已存在则 fetch 后 checkout 分支）并执行 pnpm install
  update      git pull 后 pnpm install
  start       后台启动: cd 源码目录 && pnpm paperclipai run（日志 ${LOG_FILE}）；若无实例配置则先 onboard --yes；默认校验 /instance/scheduler-heartbeats
  run         前台启动: 同 start 的准备与端口环境，但终端附着、不写 PID；后台已在跑时拒绝
  fix-embedded-postgres [版本]  在源码根执行 pnpm add，安装/固定 @embedded-postgres/<本机平台>（可传降级版本，或设 PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_VERSION）
  onboard     首次配置（交互）；NONINTERACTIVE=1 时执行 onboard --yes
  stop        停止进程
  restart     stop 后 start
  status      PID 与 HTTP 健康检查 http://0.0.0.0:${PAPERCLIP_PORT}/instance/scheduler-heartbeats
  uninstall   停止进程并删除 ${PAPERCLIP_SERVICE_HOME}（不可逆，有确认）

说明: 上游在无实例 config（默认 ${PAPERCLIP_HOME}/instances/<id>/config.json）且非 TTY 时不会自动 onboard；start 会尝试用 script(1)+onboard --yes 生成配置。
      若仍失败，请在终端执行: cd ${PAPERCLIP_SRC} && pnpm paperclipai onboard
      默认工作区目录: ${PAPERCLIP_WORKSPACE}（见 PAPERCLIP_WORKSPACE）；start 会尽量将 instances/.../workspaces 符号链接到该目录。
USAGE
}

paperclip_instance_config_json() {
  local root="${PAPERCLIP_HOME}"
  local id="${PAPERCLIP_INSTANCE_ID:-default}"
  echo "${root}/instances/${id}/config.json"
}

paperclip_instance_workspaces_dir() {
  local root="${PAPERCLIP_HOME}"
  local id="${PAPERCLIP_INSTANCE_ID:-default}"
  echo "${root}/instances/${id}/workspaces"
}

# 将上游实例下的 workspaces 指到 PAPERCLIP_WORKSPACE，便于在 ~/opt/paperclip/workspace 下集中存放各 agent 工作目录
ensure_paperclip_workspaces_symlink() {
  mkdir -p "${PAPERCLIP_WORKSPACE}"
  local ws target
  ws="$(paperclip_instance_workspaces_dir)"
  target="$(cd "${PAPERCLIP_WORKSPACE}" && pwd -P)"
  mkdir -p "$(dirname "$ws")" 2>/dev/null || true
  if [[ -L "$ws" ]]; then
    return 0
  fi
  if [[ -d "$ws" ]]; then
    if [[ -z "$(ls -A "$ws" 2>/dev/null)" ]]; then
      rmdir "$ws" 2>/dev/null || true
    else
      echo "[INFO] ${ws} 非空，未改为符号链接；仍使用上游默认路径。可清空后重启本脚本以链接到 ${target}。" >&2
      return 0
    fi
  fi
  if [[ -e "$ws" ]]; then
    return 0
  fi
  ln -sfn "$target" "$ws"
  echo "==> workspaces 已链接: ${ws} -> ${target}" >&2
}

paperclip_export_runtime_env() {
  export HOST=0.0.0.0
  export PORT="${PAPERCLIP_PORT}"
  export PAPERCLIP_HOME
  export PAPERCLIP_WORKSPACE
}

# 无配置时：在伪终端下执行 onboard --yes；若上游在 onboard 结束后仍常驻监听，在检测到 config 出现后结束该进程
ensure_paperclip_instance_config() {
  local cfg
  cfg="$(paperclip_instance_config_json)"
  if [[ -f "$cfg" ]]; then
    return 0
  fi
  command -v script >/dev/null 2>&1 || die "缺少 script(1)，无法在无 TTY 下生成配置。请在本机终端执行: cd ${PAPERCLIP_SRC} && pnpm paperclipai onboard"

  echo "==> 未找到实例配置: ${cfg}" >&2
  echo "==> 正在非交互生成默认配置（pnpm paperclipai onboard --yes）…" >&2

  local op_log
  op_log="$(mktemp "${TMPDIR:-/tmp}/nlt-paperclip-onboard.XXXXXX")"
  (
    cd "${PAPERCLIP_SRC}" || exit 1
    paperclip_export_runtime_env
    if script -qec "exit 0" /dev/null 2>/dev/null; then
      exec script -qec "cd \"${PAPERCLIP_SRC}\" && export PORT=\"${PORT}\" PAPERCLIP_HOME=\"${PAPERCLIP_HOME}\" PAPERCLIP_WORKSPACE=\"${PAPERCLIP_WORKSPACE}\" && pnpm paperclipai onboard --yes" /dev/null
    else
      exec script -q /dev/null bash -c "cd \"${PAPERCLIP_SRC}\" && export PORT=\"${PORT}\" PAPERCLIP_HOME=\"${PAPERCLIP_HOME}\" PAPERCLIP_WORKSPACE=\"${PAPERCLIP_WORKSPACE}\" && pnpm paperclipai onboard --yes"
    fi
  ) >>"${op_log}" 2>&1 &
  local opid=$!
  local waited=0
  while (( waited < 180 )); do
    if [[ -f "$cfg" ]]; then
      kill "$opid" 2>/dev/null || true
      wait "$opid" 2>/dev/null || true
      cat "${op_log}" >>"${LOG_FILE}"
      rm -f "${op_log}"
      echo "==> 已生成配置: ${cfg}" >&2
      return 0
    fi
    if ! kill -0 "$opid" 2>/dev/null; then
      wait "$opid" 2>/dev/null || true
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  kill "$opid" 2>/dev/null || true
  wait "$opid" 2>/dev/null || true
  cat "${op_log}" >>"${LOG_FILE}"
  rm -f "${op_log}"

  if [[ -f "$cfg" ]]; then
    echo "==> 已生成配置: ${cfg}" >&2
    return 0
  fi
  die "仍未生成 ${cfg}。请在**交互终端**执行: cd ${PAPERCLIP_SRC} && pnpm paperclipai onboard   然后重试: $0 start"
}

ensure_dirs() {
  mkdir -p "${PAPERCLIP_RUN_DIR}" "${PAPERCLIP_LOG_DIR}" "${PAPERCLIP_WORKSPACE}"
}

die() { echo "错误: $*" >&2; exit 1; }

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

read_pid() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$PID_FILE" || true
}

paperclip_locate_embedded_postgres_pkgjson() {
  if [[ ! -d "${PAPERCLIP_SRC}/node_modules" ]]; then
    echo ""
    return 1
  fi
  find "${PAPERCLIP_SRC}/node_modules" -path '*/embedded-postgres/package.json' 2>/dev/null | head -1
}

# 输出一行: 包名<TAB>embedded-postgres 对该平台的 optionalDependencies 条目（版本或范围）
paperclip_embedded_platform_pkg_tab_spec() {
  local ep_json="$1"
  node -e '
const fs = require("fs");
const epJson = process.argv[1];
const overridePkg = process.env.PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_PKG || "";
if (!epJson || !fs.existsSync(epJson)) {
  console.error("embedded-postgres package.json not found:", epJson);
  process.exit(2);
}
const j = JSON.parse(fs.readFileSync(epJson, "utf8"));
const opt = j.optionalDependencies || {};
const { platform, arch } = process;
let suffix = platform + "-" + arch;
if (platform === "win32") suffix = arch === "arm64" ? "win32-arm64" : "win32-x64";
let name = "@embedded-postgres/" + suffix;
if (overridePkg) name = overridePkg;
let ver = opt[name];
if (!ver) {
  if (overridePkg) {
    process.stdout.write(name + "\t\n");
    process.exit(0);
  }
  const keys = Object.keys(opt).filter((k) => k.startsWith("@embedded-postgres/"));
  console.error("No optionalDependency for " + name + ". Available: " + (keys.join(", ") || "(none)"));
  process.exit(3);
}
process.stdout.write(name + "\t" + ver + "\n");
' "$ep_json"
}

paperclip_curl_health_http_code() {
  curl -sS -o /dev/null -w '%{http_code}' -m 3 "http://0.0.0.0:${PAPERCLIP_PORT}/instance/scheduler-heartbeats" 2>/dev/null || echo "000"
}

# 返回 0=健康；1=超时仍存活；2=进程已退出
paperclip_wait_ready() {
  local pid="$1"
  local max="${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}"
  local i=0
  while (( i < max )); do
    if ! process_alive "$pid"; then
      return 2
    fi
    if command -v curl >/dev/null 2>&1; then
      local code
      code="$(paperclip_curl_health_http_code)"
      if [[ "$code" == "200" ]]; then
        return 0
      fi
    elif (( i >= 8 )); then
      echo "[WARN] 未安装 curl，无法在启动后校验 /instance/scheduler-heartbeats；进程仍存活则视为启动成功。" >&2
      return 0
    fi
    sleep 1
    i=$((i + 1))
  done
  return 1
}

paperclip_start_failure_hints() {
  echo "---- 最近日志（${LOG_FILE}）----" >&2
  tail -n 80 "${LOG_FILE}" 2>/dev/null >&2 || true
  if grep -q "ERR_MODULE_NOT_FOUND" "${LOG_FILE}" 2>/dev/null && grep -q "embedded-postgres" "${LOG_FILE}" 2>/dev/null; then
    echo "" >&2
    echo "提示: 疑似缺少或无法解析 @embedded-postgres/<平台> 可选依赖。可尝试:" >&2
    echo "  $0 fix-embedded-postgres" >&2
    echo "（若上游 optional 范围在 registry 无解，脚本会自动改用该包的 dist-tag latest）" >&2
    echo "或指定显式版本:" >&2
    echo "  $0 fix-embedded-postgres 18.1.0-beta.15" >&2
    echo "  PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_VERSION=<版本> $0 fix-embedded-postgres" >&2
  fi
}

require_git() {
  command -v git >/dev/null 2>&1 || die "需要 git"
}

require_node() {
  command -v node >/dev/null 2>&1 || die "需要 Node.js 20+（https://nodejs.org/）"
  local major
  major="$(node -p 'parseInt(process.versions.node.split(".")[0], 10)')"
  if (( major < 20 )); then
    die "需要 Node.js 20+，当前: $(node --version)"
  fi
}

ensure_pnpm() {
  if command -v pnpm >/dev/null 2>&1; then
    return 0
  fi
  if command -v corepack >/dev/null 2>&1; then
    echo "启用 corepack 并准备 pnpm …" >&2
    corepack enable
    corepack prepare pnpm@9.15.0 --activate
  fi
  command -v pnpm >/dev/null 2>&1 || die "需要 pnpm 9+（可: corepack enable && corepack prepare pnpm@9 --activate）"
}

clone_or_update_source() {
  require_git
  local parent
  parent="$(dirname "$PAPERCLIP_SRC")"
  mkdir -p "$parent"
  if [[ ! -d "${PAPERCLIP_SRC}/.git" ]]; then
    if [[ -e "$PAPERCLIP_SRC" ]]; then
      die "路径已存在且非 git 仓库: ${PAPERCLIP_SRC}"
    fi
    echo "==> git clone ${PAPERCLIP_REPO_URL} -> ${PAPERCLIP_SRC}（分支 ${PAPERCLIP_GIT_BRANCH}）" >&2
    if ! git clone --depth 1 --branch "${PAPERCLIP_GIT_BRANCH}" "${PAPERCLIP_REPO_URL}" "${PAPERCLIP_SRC}"; then
      git clone "${PAPERCLIP_REPO_URL}" "${PAPERCLIP_SRC}"
      git -C "${PAPERCLIP_SRC}" checkout "${PAPERCLIP_GIT_BRANCH}"
    fi
  else
    echo "==> 更新源码: ${PAPERCLIP_SRC}" >&2
    git -C "${PAPERCLIP_SRC}" fetch origin "${PAPERCLIP_GIT_BRANCH}" 2>/dev/null || git -C "${PAPERCLIP_SRC}" fetch origin
    git -C "${PAPERCLIP_SRC}" checkout "${PAPERCLIP_GIT_BRANCH}" 2>/dev/null || true
    git -C "${PAPERCLIP_SRC}" pull --ff-only origin "${PAPERCLIP_GIT_BRANCH}" 2>/dev/null \
      || git -C "${PAPERCLIP_SRC}" pull --ff-only || true
  fi
  [[ -f "${PAPERCLIP_SRC}/package.json" ]] || die "克隆后未找到 package.json: ${PAPERCLIP_SRC}"
}

cmd_install() {
  require_node
  ensure_pnpm
  ensure_dirs
  clone_or_update_source
  echo "==> pnpm install（${PAPERCLIP_SRC}）…" >&2
  (cd "${PAPERCLIP_SRC}" && pnpm install)
  echo "安装完成。执行: $0 start（或 PAPERCLIP_SERVICE_HOME=… $0 start）"
}

cmd_update() {
  require_node
  ensure_pnpm
  [[ -d "${PAPERCLIP_SRC}/.git" ]] || die "未找到源码目录，请先 install"
  require_git
  echo "==> 强制更新: 丢弃本地变更…" >&2
  git -C "${PAPERCLIP_SRC}" fetch origin
  git -C "${PAPERCLIP_SRC}" reset --hard "origin/${PAPERCLIP_GIT_BRANCH}"
  git -C "${PAPERCLIP_SRC}" clean -fd
  echo "==> pnpm install …" >&2
  (cd "${PAPERCLIP_SRC}" && pnpm install)
  echo "更新完成。"
}

cmd_onboard() {
  require_node
  ensure_pnpm
  ensure_dirs
  [[ -d "${PAPERCLIP_SRC}" && -f "${PAPERCLIP_SRC}/package.json" ]] || die "未安装源码，请先: $0 install"
  pushd "${PAPERCLIP_SRC}" >/dev/null
  paperclip_export_runtime_env
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    pnpm paperclipai onboard --yes "$@"
  else
    pnpm paperclipai onboard "$@"
  fi
  popd >/dev/null
  if [[ -f "$(paperclip_instance_config_json)" ]]; then
    ensure_paperclip_workspaces_symlink || true
  fi
}

paperclip_pnpm_add_platform_pkg_at() {
  local pkg="$1"
  local ver="$2"
  (
    cd "${PAPERCLIP_SRC}" || exit 1
    if [[ -f pnpm-workspace.yaml ]] || [[ -f pnpm-workspace.yml ]]; then
      pnpm add "${pkg}@${ver}" -w
    else
      pnpm add "${pkg}@${ver}"
    fi
  )
}

cmd_fix_embedded_postgres() {
  require_node
  ensure_pnpm
  [[ -d "${PAPERCLIP_SRC}" && -f "${PAPERCLIP_SRC}/package.json" ]] || die "未安装源码，请先: $0 install"
  local ep_json want line pkg spec reg_ver
  ep_json="$(paperclip_locate_embedded_postgres_pkgjson)"
  [[ -n "$ep_json" && -f "$ep_json" ]] || die "未找到 embedded-postgres。请先: cd ${PAPERCLIP_SRC} && pnpm install"
  line="$(paperclip_embedded_platform_pkg_tab_spec "$ep_json")" || die "无法解析平台包（可设置 PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_PKG）"
  pkg="${line%%$'\t'*}"
  spec="${line#*$'\t'}"
  want="${1:-${PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_VERSION:-$spec}}"
  [[ -n "$want" ]] || die "未指定版本。请执行: $0 fix-embedded-postgres <版本> 或设置 PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_VERSION（使用 PAPERCLIP_EMBEDDED_POSTGRES_PLATFORM_PKG 时需显式版本）"
  echo "==> 在 ${PAPERCLIP_SRC} 安装平台二进制: ${pkg}@${want}" >&2
  set +e
  paperclip_pnpm_add_platform_pkg_at "$pkg" "$want"
  local add_rc=$?
  set -e
  if [[ "$add_rc" -eq 0 ]]; then
    echo "完成。可执行: $0 start"
    return 0
  fi
  if [[ "${PAPERCLIP_EMBEDDED_POSTGRES_NO_REGISTRY_FALLBACK:-}" == "1" ]]; then
    die "pnpm add 失败（已设置 PAPERCLIP_EMBEDDED_POSTGRES_NO_REGISTRY_FALLBACK=1，未尝试 registry 回退）。可改用显式版本: $0 fix-embedded-postgres <版本>"
  fi
  echo "==> 警告: ${pkg}@${want} 安装失败（常见原因：embedded-postgres 的 optional 范围指向尚未发布的平台包版本，例如 registry 仅有 beta.15 而范围为 ^beta.16）。" >&2
  echo "==> 正在查询 ${pkg} 的 dist-tag latest …" >&2
  reg_ver="$(cd "${PAPERCLIP_SRC}" && pnpm view "$pkg" version 2>/dev/null | tail -1)"
  reg_ver="$(echo "$reg_ver" | tr -d '[:space:]')"
  [[ -n "$reg_ver" ]] || die "仍无法安装，且 pnpm view ${pkg} version 无结果。请手动: pnpm view ${pkg} versions"
  if [[ "$reg_ver" == "$want" ]]; then
    die "pnpm add 已失败且 registry latest 与尝试版本相同。请查看上方 pnpm 报错或清理 node_modules 后重试 pnpm install。"
  fi
  echo "==> 改用 registry latest: ${pkg}@${reg_ver}" >&2
  paperclip_pnpm_add_platform_pkg_at "$pkg" "$reg_ver" || die "pnpm add ${pkg}@${reg_ver} 仍失败"
  echo "完成（已使用 registry 最新平台包 ${reg_ver}）。若运行期与 embedded-postgres 不兼容，请改用: $0 fix-embedded-postgres <显式版本>。可执行: $0 start"
}

cmd_start() {
  require_node
  ensure_pnpm
  [[ -d "${PAPERCLIP_SRC}" && -f "${PAPERCLIP_SRC}/package.json" ]] || die "未安装源码，请先: $0 install"
  ensure_dirs
  ensure_paperclip_instance_config
  ensure_paperclip_workspaces_symlink
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Paperclip 已在运行（PID ${existing}）。如需重启: $0 restart" >&2
    exit 1
  fi
  rm -f "$PID_FILE"
  echo "==> 启动 Paperclip（pnpm paperclipai run），日志: ${LOG_FILE}" >&2
  echo "    默认 UI/API: http://0.0.0.0:${PAPERCLIP_PORT}" >&2
  pushd "${PAPERCLIP_SRC}" >/dev/null
  paperclip_export_runtime_env
  nohup pnpm paperclipai run >>"${LOG_FILE}" 2>&1 &
  local cpid=$!
  echo "$cpid" >"$PID_FILE"
  popd >/dev/null
  sleep 1
  if ! process_alive "$cpid"; then
    rm -f "$PID_FILE"
    paperclip_start_failure_hints
    die "进程已退出，启动失败。"
  fi
  if [[ "${PAPERCLIP_SKIP_START_HEALTH_CHECK:-}" == "1" ]]; then
    echo "已启动 PID ${cpid}（已跳过 HTTP 健康检查）"
    return 0
  fi
  echo "==> 等待 http://0.0.0.0:${PAPERCLIP_PORT}/instance/scheduler-heartbeats 就绪（最长 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s）…" >&2
  local wr=0
  paperclip_wait_ready "$cpid" || wr=$?
  if [[ "$wr" -eq 0 ]]; then
    echo "已启动 PID ${cpid}，健康检查通过。"
    return 0
  fi
  if [[ "$wr" -eq 2 ]]; then
    rm -f "$PID_FILE"
    paperclip_start_failure_hints
    die "进程已退出，启动失败。"
  fi
  echo "错误: 在 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s 内未通过健康检查；进程可能仍在运行（PID ${cpid}）。" >&2
  paperclip_start_failure_hints
  die "启动校验失败。"
}

cmd_run() {
  require_node
  ensure_pnpm
  [[ -d "${PAPERCLIP_SRC}" && -f "${PAPERCLIP_SRC}/package.json" ]] || die "未安装源码，请先: $0 install"
  ensure_dirs
  ensure_paperclip_instance_config
  ensure_paperclip_workspaces_symlink
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    die "Paperclip 已在后台运行（PID ${existing}）。请先执行 $0 stop，再使用 run 前台调试。"
  fi
  echo "==> 前台启动 Paperclip（pnpm paperclipai run），日志输出至本终端；按 Ctrl+C 结束；不写 PID。" >&2
  echo "    默认 UI/API: http://0.0.0.0:${PAPERCLIP_PORT}" >&2
  pushd "${PAPERCLIP_SRC}" >/dev/null
  paperclip_export_runtime_env
  exec pnpm paperclipai run
}

cmd_stop() {
  local pid
  pid="$(read_pid)"
  if [[ -z "$pid" ]]; then
    echo "未找到 PID 文件，视为未启动。" >&2
    rm -f "$PID_FILE"
    return 0
  fi
  if ! process_alive "$pid"; then
    echo "PID ${pid} 不存在，清理 PID 文件。"
    rm -f "$PID_FILE"
    return 0
  fi
  if [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    gum confirm "停止 Paperclip（PID ${pid}）？" || exit 0
  fi
  kill -9 "$pid" 2>/dev/null || true
  local w=0
  while process_alive "$pid" && (( w < 30 )); do
    sleep 1
    w=$((w + 1))
  done
  if process_alive "$pid"; then
    kill -KILL "$pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
  echo "已停止。"
}

cmd_restart() {
  cmd_stop || true
  cmd_start
}

cmd_status() {
  local pid
  pid="$(read_pid)"
  echo "PAPERCLIP_SRC=${PAPERCLIP_SRC}"
  echo "PAPERCLIP_SERVICE_HOME=${PAPERCLIP_SERVICE_HOME}"
  echo "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
  echo "PAPERCLIP_WORKSPACE=${PAPERCLIP_WORKSPACE}"
  local _ws
  _ws="$(paperclip_instance_workspaces_dir)"
  if [[ -L "$_ws" ]]; then
    echo "instances/.../workspaces -> $(readlink "$_ws" 2>/dev/null || true)"
  elif [[ -d "$_ws" ]]; then
    echo "instances/.../workspaces=${_ws}（目录）"
  fi
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    echo "状态: 运行中 PID ${pid}"
  else
    echo "状态: 未运行"
    rm -f "$PID_FILE"
  fi
  if command -v curl >/dev/null 2>&1; then
    echo ""
    echo "==> GET http://0.0.0.0:${PAPERCLIP_PORT}/instance/scheduler-heartbeats"
    curl -sS -m 3 "http://0.0.0.0:${PAPERCLIP_PORT}/instance/scheduler-heartbeats" || echo "（无法连接，可能未启动或端口不同）"
    echo ""
  fi
}

cmd_uninstall() {
  cmd_stop || true
  echo "将删除目录: ${PAPERCLIP_SERVICE_HOME}" >&2
  if [[ -t 0 ]]; then
    gum confirm "确认永久删除上述目录（不含 PAPERCLIP_HOME=${PAPERCLIP_HOME} 下数据，仅服务安装根）？" || exit 0
  else
    [[ "${PAPERCLIP_UNINSTALL_YES:-}" == "1" ]] || die "非交互卸载请设置 PAPERCLIP_UNINSTALL_YES=1"
  fi
  local hp ap
  hp="$(cd "$HOME" && pwd -P)"
  ap="$(cd "${PAPERCLIP_SERVICE_HOME}" 2>/dev/null && pwd -P)" || ap="${PAPERCLIP_SERVICE_HOME}"
  if [[ "$ap" == "/" || "$ap" == "$hp" ]]; then
    die "拒绝删除根目录或 \$HOME"
  fi
  rm -rf "${PAPERCLIP_SERVICE_HOME}"
  echo "已删除 ${PAPERCLIP_SERVICE_HOME}（实例数据在 PAPERCLIP_HOME=${PAPERCLIP_HOME}，需自行清理）"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    onboard) cmd_onboard "$@" ;;
    fix-embedded-postgres) cmd_fix_embedded_postgres "$@" ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: ${cmd}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

interactive_main() {
  gum style --bold --foreground 212 "Paperclip 本地服务（源码: paperclipai/paperclip）"
  gum style "PAPERCLIP_SRC=${PAPERCLIP_SRC}"
  gum style "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
  echo ""
  set +e
  while true; do
    local pick
    pick="$(gum choose --header "选择操作（取消退出）" \
      "install" "update" "onboard" "fix-embedded-postgres" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
    esac
    ( dispatch "$pick" )
    echo ""
  done
  set -e
}

main() {
  if [[ $# -gt 0 ]]; then
    case "$1" in
      help | -h | --help)
        dispatch "$@"
        return 0
        ;;
    esac
  fi
  _nlt_ensure_gum || exit 1
  if [[ $# -eq 0 ]]; then
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
