#!/usr/bin/env bash
# Paperclip 本机服务。CLI 安装、升级、诊断和后台服务全部交给 Paperclip 官方命令，
# 本脚本只做两件上游没有、或在本仓环境里必须补的事：中文封装，以及没有服务管理器
# 的机器上的后台托管。
#
# 上游（本脚本对齐 paperclipai 2026.1008.0-canary.4 的 CLI 表面）:
#   https://github.com/paperclipai/paperclip
#   npm 包 paperclipai。managed install 的产物布局：
#     ${PAPERCLIP_HOME}/cli/installs/npm/<版本>   payload（整棵依赖树，实测约 1.4G）
#     ${PAPERCLIP_HOME}/cli/current               指向当前版本的符号链接
#     ~/.local/bin/paperclipai                    shim
#     ${PAPERCLIP_HOME}/instances/<实例>/config.json  实例配置
#
# 为什么不走官方 install.sh：
#   官方文档推荐 `curl -O https://paperclip.ing/install.sh` + sha256 校验，但 paperclip.ing
#   上那份是过期快照（落后于 master 的 #11792，上游 issue #10739 在跟踪）：给 CLI 传了
#   `install` 并不接受的 --no-prompt（交接直接失败）、MIN_NODE_MAJOR=20（上游已要求
#   24.11+）、用了 ${value,,} 这种 bash 4 语法（macOS 自带 bash 3.2 上崩）、第三方脚本
#   无 SHA 固定。而 master 上那份正确的 scripts/install.sh 没有发布 .sha256（404），
#   无从校验。它的 Node 引导还会在见到 ~/.nvm 时直接 fail（"nvm was detected..."）。
#   反过来，只要 PATH 上已有 node 24+，install.sh 的 Node 引导块会被整个跳过，它就只
#   剩下面这一行 —— 也正是本脚本 cmd_install 执行的那条：
#     npx --yes --registry https://registry.npmjs.org paperclipai@latest install --yes
#   所以这里直接驱动官方 CLI（INSTALLING.md 的 "Install Sources" 记载的正式路径），
#   终态与 install.sh 完全一致，但不引入一个过期、且在装了 nvm 的机器上根本跑不起来
#   的脚本依赖。
#
# PID 归属：
#   有服务管理器（Linux systemd --user / macOS LaunchAgent）时 PID 完全由官方
#   `paperclipai service` 持有，本脚本不写 PID 文件，start/stop/restart/status/logs
#   只是转调。但 systemd --user 需要一个用户级 systemd 实例，容器、WSL1 和没有登录
#   会话的终端里不存在；官方 service start/stop/status 在那种机器上只打印一条 reason
#   就以 0 退出（commands/service.ts 的 resolveManager 返回 null 而非抛错），对调用方
#   是彻底的「假成功」—— 返回 0、什么都没启动。官方给出的唯一出路是 `paperclipai run`，
#   而那是前台的、不自守护的进程。因此这种机器上由本脚本用 nohup + PID 文件托管一条
#   后台通道，让那五个命令在容器里也是真的。运行时文件落在
#   ${PAPERCLIP_HOME}/run/<实例>/，刻意不放仓库内的 .run/：fundeploy 以 apt / Homebrew /
#   ~/.local/fundeploy 形式分发，运行时根本不存在「仓库工作区」。
#
# 为什么 upgrade / rollback / uninstall 不直接转调上游同名命令：
#   - 官方 `update` 只会重启「服务管理器下的」那个实例，看不见本脚本 nohup 托管的进程；
#     不先停就会在服务运行中替换 payload。
#   - 官方 `service uninstall` 在无服务管理器的机器上同样「假成功」，不先收掉托管进程
#     就会把正在运行的 payload 删掉。
#   - canary 机器上 `--latest` 指向的 stable 往往比当前版本旧，官方 update 把这种情况
#     当显式降级、要求 -y 确认，不处理的话非交互环境里会卡住。
#   - 官方 update 每次都会先备一次数据库（本机 370M、耗时数十秒），而它换的是 payload、
#     不动数据目录；所以统一传 --no-backup，备份提成单独的 backup 命令按需执行。
#
# 依赖: Node.js 24.11+（只有 npx 引导首次安装那一步需要；此后 shim 用绝对路径钉死了
#   自己那份 node）、curl；可选 nc（端口探测，缺了用 /dev/tcp）。
#
# 用法：
#   ./setup.sh                      # gum 菜单
#   ./setup.sh install              # 官方 managed install（正式版）
#   ./setup.sh install-canary       # 官方 managed install（开发版）
#   ./setup.sh check [canary|prod]  # 只看有没有新版，不改动任何东西
#   ./setup.sh upgrade [canary|prod]# 停服 → 官方 update → doctor --repair → 重启（不备份）
#   ./setup.sh rollback             # 回退到 install store 里保留的上一份 payload
#   ./setup.sh backup               # 备份数据库（官方 db:backup）
#   ./setup.sh doctor [--repair]    # 官方诊断（--repair 尝试自动修复）
#   ./setup.sh health               # 官方健康检查（不可达时返回非 0）
#   ./setup.sh channels             # 查看发布通道和当前跟随的通道
#   ./setup.sh start / stop / restart / status / logs / run
#   ./setup.sh uninstall            # 卸载服务和 CLI，保留数据目录
#
# 环境变量：
#   PAPERCLIP_HOME                  数据目录，默认 ~/.paperclip
#   PAPERCLIP_INSTANCE_ID           实例 id，默认 default
#   PAPERCLIP_FALLBACK=off          关掉无服务管理器时的后台托管，维持官方行为
#   PAPERCLIP_HTTPS_PROXY           npm 下载代理，如 http://127.0.0.1:7890
#   PAPERCLIP_NO_PROXY              配合上一项的排除列表
#   PAPERCLIP_NETWORK_HINT=0        隐藏「依赖树约 1.4G」的提示
#   PAPERCLIP_INSTALL_VIA_NPX=1     强制 install 走 npx（默认复用已装 CLI 以省一份下载）
#   PAPERCLIP_NPM_PREFER_OFFLINE=1  优先用 npm cache，减少重装同版本时的网络往返
#   PAPERCLIP_NPM_FETCH_RETRIES / _FETCH_TIMEOUT 等  覆盖 npm 取回重试与超时
#   NONINTERACTIVE=1                onboard 自动追加 --yes；降级需配合下面两个之一
#   PAPERCLIP_UPGRADE_YES=1         非交互下确认降级
#   PAPERCLIP_UNINSTALL_YES=1       非交互下确认卸载

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

PAPERCLIP_HOME="${PAPERCLIP_HOME:-${HOME}/.paperclip}"
PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID:-default}"
# 官方源。用途只有一个：npx 引导那一步的 registry。除非官方换域名，不要改 ——
# managed install 自己也把它钉死（见 paperclip_prepare_npm_env 的注释）。
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
# 受限网络下的 npm 取回加固。npm 默认 fetch-retries=2、fetch-timeout=5min，而本项目
# 单个 tarball 可达 370M（@openai/codex-linux-x64），直连跨境链路极易在中途断流，
# 一旦失败整棵 1.4G 依赖树都要重来，因此默认把重试次数和超时都放宽。
PAPERCLIP_NPM_FETCH_RETRIES="${PAPERCLIP_NPM_FETCH_RETRIES:-5}"
PAPERCLIP_NPM_FETCH_RETRY_MINTIMEOUT="${PAPERCLIP_NPM_FETCH_RETRY_MINTIMEOUT:-20000}"
PAPERCLIP_NPM_FETCH_RETRY_MAXTIMEOUT="${PAPERCLIP_NPM_FETCH_RETRY_MAXTIMEOUT:-180000}"
PAPERCLIP_NPM_FETCH_TIMEOUT="${PAPERCLIP_NPM_FETCH_TIMEOUT:-1200000}"
PAPERCLIP_HTTPS_PROXY="${PAPERCLIP_HTTPS_PROXY:-}"
PAPERCLIP_NO_PROXY="${PAPERCLIP_NO_PROXY:-}"
PAPERCLIP_NODE_MIN_VERSION="${PAPERCLIP_NODE_MIN_VERSION:-24.11.0}"
PAPERCLIP_NODE_MIN_MAJOR="${PAPERCLIP_NODE_MIN_MAJOR:-24}"
# 升级后要等 embedded PostgreSQL 冷启动、数据库迁移和服务就绪，低配或受限网络的机器
# 60 秒常不够，超时即 die 会把「还在启动」误报成「升级失败」，故放宽到 180 秒。
PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC="${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC:-180}"
PAPERCLIP_CONFIG_PATH="${PAPERCLIP_HOME}/instances/${PAPERCLIP_INSTANCE_ID}/config.json"

# ===== 无服务管理器时的后台托管 =====
# ${PAPERCLIP_HOME} 下官方只用 cli/ 和 instances/，run/ 不会撞上它。
PAPERCLIP_RUN_DIR="${PAPERCLIP_RUN_DIR:-${PAPERCLIP_HOME}/run/${PAPERCLIP_INSTANCE_ID}}"
PAPERCLIP_PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip.pid"
PAPERCLIP_LOG_FILE="${PAPERCLIP_LOG_FILE:-${PAPERCLIP_RUN_DIR}/paperclip.log}"
# auto：无服务管理器时自动接管；off：维持官方行为（只报不支持并退出）。
PAPERCLIP_FALLBACK="${PAPERCLIP_FALLBACK:-auto}"
PAPERCLIP_START_HEALTH_TIMEOUT_SEC="${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-180}"
# 与官方 systemd 单元的 TimeoutStopSec=300 对齐：SIGTERM 后服务要排空正在跑的
# heartbeat run，提前 KILL 会打断执行中的任务。
PAPERCLIP_STOP_TIMEOUT_SEC="${PAPERCLIP_STOP_TIMEOUT_SEC:-300}"
PAPERCLIP_LOG_TAIL_LINES="${PAPERCLIP_LOG_TAIL_LINES:-200}"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

安装与版本:
  install          官方 managed install（正式版；别名 install-prod）
  install-canary   官方 managed install（开发版）
  check [canary|prod]   只查有没有新版，不改动任何东西（官方 update --check --json）
  upgrade [canary|prod] 停服 → 官方 update → doctor --repair → 重启 → 健康检查
                        默认 canary；别名 update。降级需确认（见 PAPERCLIP_UPGRADE_YES）
                        不做数据库备份，需要的话先执行 backup
  rollback         回退到 install store 里保留的上一份 payload（官方 update --rollback）
  backup           备份数据库（官方 db:backup）；embedded PostgreSQL 模式下要求服务在跑
                   （它是服务的子进程），外部数据库与服务生命周期无关、停着也能备
  channels         查看发布通道（stable / beta / nightly / canary）和当前跟随的通道

诊断与配置:
  doctor [--repair]     官方诊断；--repair 尝试自动修复（配置、store、shim、PATH、目录权限）
  health [--json]       官方健康检查，API 不可达时返回非 0
  onboard               首次配置；NONINTERACTIVE=1 时默认追加 --yes
  service-install [--enable-linger] [--no-start-on-login] [--no-start-now]
                        注册后台服务并设置开机自启；无登录会话的机器需 --enable-linger

插件:
  plugin list                  列出 awesome-paperclip 收录的插件
  plugin install [npm-package] 安装插件；不指定包名时从清单选择
  plugin installed             列出当前已安装的插件

生命周期:
  start             后台启动并等待 /api/health 通过
  run               使用 paperclipai run 前台启动
  stop              停止后台服务
  restart [--wait]  热重启；未在运行时自动改为 start
  status [--json]   查看 supervisor、版本与健康状态
  logs [-f] [-n N]  查看服务日志
  uninstall         卸载官方服务和 CLI，保留 ${PAPERCLIP_HOME} 数据

说明:
  - Linux 优先用 systemd --user，macOS 用 LaunchAgent。
  - systemd --user 需要一个用户级 systemd 实例；容器、WSL1 或没有登录会话的终端里
    不存在。这种机器上 start/stop/restart/status/logs 自动改由 fundeploy 用
    nohup + PID 文件托管（PID ${PAPERCLIP_PID_FILE}、日志 ${PAPERCLIP_LOG_FILE}），
    代价是不开机自启。想走官方 supervisor 就先在有登录会话的 shell 执行
    service-install --enable-linger（等价于 loginctl enable-linger）；
    设 PAPERCLIP_FALLBACK=off 可关掉这条兜底通道。
  - 配置由 paperclipai onboard / configure 管理。
  - PAPERCLIP_INSTANCE_ID 默认为 ${PAPERCLIP_INSTANCE_ID}。
  - start 和 upgrade 完成后检查 PostgreSQL、Paperclip 端口及 /api/health。

受限网络（install / upgrade 下载约 1.4G）:
  - 官方 managed install 把 registry 钉死在 registry.npmjs.org 并自己改写 npmrc，这是
    刻意的防投毒设计（有上游测试守着），换镜像对它无效。能透进去的只有环境变量，
    因此这里只提供代理和取回参数两类开关：
  - PAPERCLIP_HTTPS_PROXY     npm 代理，如 http://127.0.0.1:7890；留空为直连
  - PAPERCLIP_NO_PROXY        配合上一项的排除列表
  - PAPERCLIP_NETWORK_HINT=0  隐藏直连提示
  - PAPERCLIP_INSTALL_VIA_NPX=1  强制 install 走 npx（默认复用已有 CLI 以省一份下载）
  - PAPERCLIP_NPM_PREFER_OFFLINE=1  优先用 npm cache，减少重装同版本时的网络往返
  - PAPERCLIP_NPM_FETCH_RETRIES / _FETCH_TIMEOUT 等可覆盖 npm 取回重试与超时
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

# ------------------------------------------------------------------ Node 运行时
#
# Paperclip 自己要求 Node 24.11+，但 managed install 装完后的 shim 已经用绝对路径把
# 自己那份 node 钉死了：
#   export PATH='<nvm>/versions/node/v24.x/bin':"$PATH"
#   exec '<nvm>/versions/node/v24.x/bin/node' '<payload>/dist/index.js' "$@"
# 所以经 shim 的调用自带正确的 node，PATH 上是不是旧版无关。真正需要版本门槛的只有
# npx 引导首次安装那一步（那时还没有 shim），以及 run / start 可能解析到非 shim 的
# paperclipai 的情况。本脚本自己的 node -e 小工具（解析 config.json、service status
# --json）只需要任意可用的 node，因此分成两级，避免在 status / logs / stop 这些热路径
# 上每次都去 source nvm.sh 并改写 PATH。

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

# 本脚本自己的 node -e 小工具用：只要有 node 就行，不切版本。
require_node_any() {
  command -v node >/dev/null 2>&1 || die "需要 Node.js（https://nodejs.org/）"
}

# ------------------------------------------------------------------ 配置与 CLI

paperclip_load_config() {
  PAPERCLIP_DATABASE_MODE=""
  PAPERCLIP_DATABASE_DIR="${PAPERCLIP_HOME}/instances/${PAPERCLIP_INSTANCE_ID}/db"
  PAPERCLIP_DATABASE_PORT=5432
  PAPERCLIP_SERVER_PORT=8804
  [[ -f "${PAPERCLIP_CONFIG_PATH}" ]] || return 0
  require_node_any

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
  local executable rc=0
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  paperclip_load_config
  if [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" ]]; then
    (unset DATABASE_URL; PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" "$@") || rc=$?
  else
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" "$@" || rc=$?
  fi
  # paperclip_executable 只判断 -x：payload 被删或损坏而 shim 仍在时，官方 CLI 只会
  # 抛 node 的原始堆栈。失败后补一次探测把它翻译成可执行的建议；正常路径零开销。
  # PAPERCLIP_CLI_NO_DIAGNOSE=1 关掉这次探测，供 update --check 那类「用退出码传信息、
  # 非零不代表失败」的调用使用（见 paperclip_update_check）。
  if ((rc != 0)) && [[ "${PAPERCLIP_CLI_NO_DIAGNOSE:-0}" != "1" ]] && ! "$executable" --version >/dev/null 2>&1; then
    echo "提示: ${executable} 无法运行，paperclipai 安装可能已损坏，可执行: $0 install" >&2
  fi
  return "$rc"
}

# ------------------------------------------------------------- 官方服务状态判读
#
# service start / stop / status 在检测不到服务管理器时只打印一条 reason 就以 0 退出
# （commands/service.ts 的 resolveManager 返回 null 而非抛错），对调用方是「假成功」。
# 这里统一解析 --json 输出，取出 supported / pid / message 供上层判断。
# 依次输出三行 supported / pid / message；拿不到可解析的状态时返回非 0。
# 刻意分行而非用 \t 拼接：tab 属于 IFS 空白字符，read 会把连续分隔符合并成一个，
# pid 为空时字段会整体错位（message 被读进 pid）。
paperclip_service_state() {
  local json
  json="$(paperclip_cli service status --json 2>/dev/null)" || return 1
  printf '%s' "$json" | node -e '
    const fs = require("node:fs");
    let value;
    try { value = JSON.parse(fs.readFileSync(0, "utf8")); } catch { process.exit(1); }
    const pid = Number.isInteger(value.pid) && value.pid > 0 ? String(value.pid) : "";
    console.log(value.supported === false ? "false" : "true");
    console.log(pid);
    console.log(String(value.message || "").replace(/\s+/g, " ").trim());
  ' 2>/dev/null || return 1
}

# 把 paperclip_service_state 的三行读进 PAPERCLIP_SERVICE_{SUPPORTED,PID,MESSAGE}。
# 拿不到状态时 SUPPORTED 置为 unknown —— 那种情况下不猜，交给官方命令自己报错。
paperclip_read_service_state() {
  PAPERCLIP_SERVICE_SUPPORTED=unknown
  PAPERCLIP_SERVICE_PID=""
  PAPERCLIP_SERVICE_MESSAGE=""
  local state
  state="$(paperclip_service_state)" || return 0
  # read 读到最后一行若无换行会返回非 0，但变量已赋值，故逐行 || true。
  {
    read -r PAPERCLIP_SERVICE_SUPPORTED || true
    read -r PAPERCLIP_SERVICE_PID || true
    IFS= read -r PAPERCLIP_SERVICE_MESSAGE || true
  } <<<"$state"
}

# 官方 supervisor 不可用时是否该由 fundeploy 接管；关掉兜底时直接 die。
paperclip_require_fallback() {
  [[ "${PAPERCLIP_FALLBACK}" != "off" ]] && return 0
  die "${PAPERCLIP_SERVICE_MESSAGE:-未检测到可用的服务管理器}（PAPERCLIP_FALLBACK=off）
可选做法: $0 run 前台启动，或在有登录会话的 shell 执行 $0 service-install --enable-linger"
}

# ------------------------------------------------------------------ 端口与健康

paperclip_port_open() {
  local port="$1"
  if command -v nc >/dev/null 2>&1; then
    nc -z 127.0.0.1 "$port" >/dev/null 2>&1
  else
    (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null
  fi
}

# 单次探测：端口通且 /api/health 返回 2xx。
# 刻意用 curl 而不是官方 `paperclipai health`：后者每次都要起一个 node 进程加载整个 CLI
# bundle，按秒轮询最多 180 次扛不住。`health` 作为用户命令单独暴露（cmd_health）。
paperclip_health_ok() {
  { [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || paperclip_port_open "${PAPERCLIP_DATABASE_PORT}"; } &&
    paperclip_port_open "${PAPERCLIP_SERVER_PORT}" &&
    curl --fail --silent --show-error --max-time 3 "http://127.0.0.1:${PAPERCLIP_SERVER_PORT}/api/health" >/dev/null
}

# 等端口和 /api/health 就绪；超时返回 1，由调用方决定怎么报错。
# 首探就通过时不打提示：升级编排里 cmd_start 已经等过一轮，再打一遍只是噪音；而官方
# service start 可能在服务真正就绪前就返回，所以那一轮等待本身不能省。
paperclip_wait_for_health() {
  local timeout="$1" deadline=$((SECONDS + $1)) targets="Paperclip ${PAPERCLIP_SERVER_PORT} 和 /api/health"
  paperclip_health_ok && return 0
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || targets="PostgreSQL ${PAPERCLIP_DATABASE_PORT}、${targets}"
  echo "==> 检查 ${targets}（最长 ${timeout} 秒）" >&2
  while ((SECONDS <= deadline)); do
    sleep 1
    paperclip_health_ok && return 0
  done
  return 1
}

# embedded PostgreSQL 是服务的子进程，但 nohup 托管的进程被收掉后它可能变成孤儿占住
# 5432，换 payload 或重启时就会失败。这不是 macOS 专属问题，所有平台都要收。
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

# ------------------------------------------------------------ fundeploy 后台托管

# 输出 fundeploy 托管进程的 PID；未在运行时返回 1。
paperclip_fallback_pid() {
  local pid command_line
  pid="$(_fundeploy_read_pid_file "${PAPERCLIP_PID_FILE}")"
  [[ -n "$pid" ]] || return 1
  _fundeploy_process_alive "$pid" || return 1
  # PID 会被复用，而这个文件可能是上次重启前留下的。只认命令行里确实是
  # paperclipai 的进程，否则宁可当成「没在运行」，也不能朝陌生进程发 SIGTERM。
  command_line="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command_line" == *paperclipai* ]] || return 1
  printf '%s\n' "$pid"
}

paperclip_start_fallback() {
  require_node
  local executable
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  paperclip_load_config

  local running
  if running="$(paperclip_fallback_pid)"; then
    echo "Paperclip 已由 fundeploy 在后台运行（PID ${running}，日志 ${PAPERCLIP_LOG_FILE}）。"
    return 0
  fi

  # 端口上已经有别的进程（例如一个 $0 run 的前台实例）时再起一个只会抢不到端口后
  # 自己退出，白写一份 PID 文件。
  local listener
  listener="$(_fundeploy_listener_pid_for_port "${PAPERCLIP_SERVER_PORT}")"
  [[ -z "$listener" ]] ||
    die "端口 ${PAPERCLIP_SERVER_PORT} 已被 PID ${listener} 占用（可能是 $0 run 的前台实例），请先停止后重试"

  mkdir -p "${PAPERCLIP_RUN_DIR}" || die "无法创建运行目录 ${PAPERCLIP_RUN_DIR}"
  rm -f "${PAPERCLIP_PID_FILE}"
  echo "==> ${PAPERCLIP_SERVICE_MESSAGE:-未检测到可用的服务管理器}" >&2
  echo "==> 改由 fundeploy 托管后台进程；PID ${PAPERCLIP_PID_FILE}，日志 ${PAPERCLIP_LOG_FILE}" >&2

  # 刻意不设 PAPERCLIP_SERVICE_MANAGED=1：它只用于让 run 跳过「同实例已在 supervisor
  # 下运行」的检查，而这里恰好需要保留那层保护。
  local -a launcher=(env)
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || launcher+=(-u DATABASE_URL)
  launcher+=("PAPERCLIP_HOME=${PAPERCLIP_HOME}" "PAPERCLIP_INSTANCE_ID=${PAPERCLIP_INSTANCE_ID}"
    "$executable" run --instance "${PAPERCLIP_INSTANCE_ID}")

  local pid
  pid="$(
    # cwd 对齐官方单元的 WorkingDirectory=%h：别让服务继承调用者的目录，否则
    # agent 的 fallback cwd 会随在哪儿执行 start 而变。
    cd "${HOME}" || exit 1
    # </dev/null 必须有：不断开 stdin 的话后台进程会继承调用者的终端，交互式 shell
    # 里它一读 stdin 就被 SIGTTIN 停住，看起来是「起来了但没响应」。
    nohup "${launcher[@]}" </dev/null >>"${PAPERCLIP_LOG_FILE}" 2>&1 &
    echo $!
  )" || die "启动失败"
  [[ "$pid" =~ ^[0-9]+$ ]] || die "启动失败，未取到后台进程 PID（日志 ${PAPERCLIP_LOG_FILE}）"
  echo "$pid" >"${PAPERCLIP_PID_FILE}"

  if paperclip_wait_for_health "${PAPERCLIP_START_HEALTH_TIMEOUT_SEC}"; then
    echo "Paperclip 已启动（PID ${pid}，http://127.0.0.1:${PAPERCLIP_SERVER_PORT} ），健康检查通过。"
    return 0
  fi

  if ! _fundeploy_process_alive "$pid"; then
    rm -f "${PAPERCLIP_PID_FILE}"
    echo "==> 进程已退出，日志末尾：" >&2
    tail -n 40 "${PAPERCLIP_LOG_FILE}" >&2 || true
    die "Paperclip 启动失败（完整日志: ${PAPERCLIP_LOG_FILE}）"
  fi
  echo "==> 进程仍在（PID ${pid}），日志末尾：" >&2
  tail -n 40 "${PAPERCLIP_LOG_FILE}" >&2 || true
  die "Paperclip 未在 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC} 秒内通过健康检查（可用 $0 status / logs 确认，$0 stop 停止）"
}

# 停止 fundeploy 托管的进程并清掉 PID 文件。
# --quiet：没东西可停时不出声，供官方 stop 之后的收尾调用（那条路径已经有输出了）。
paperclip_stop_fallback() {
  local quiet="" pid
  [[ "${1:-}" != "--quiet" ]] || quiet=1
  if ! pid="$(paperclip_fallback_pid)"; then
    [[ -n "$quiet" ]] || echo "Paperclip 未在运行（无 fundeploy 托管进程）。"
    rm -f "${PAPERCLIP_PID_FILE}"
    return 0
  fi
  echo "==> 停止 fundeploy 托管的 Paperclip（PID ${pid}，最长等 ${PAPERCLIP_STOP_TIMEOUT_SEC} 秒排空运行中的任务）" >&2
  _fundeploy_stop_pid "$pid" "${PAPERCLIP_STOP_TIMEOUT_SEC}"
  rm -f "${PAPERCLIP_PID_FILE}"
  echo "已停止 fundeploy 托管的 Paperclip。"
}

paperclip_status_fallback() {
  paperclip_load_config
  local pid version health="不通过"
  version="$(paperclip_cli --version 2>/dev/null | awk 'NR==1{print $1}')" || version=""
  curl --fail --silent --show-error --max-time 3 \
    "http://127.0.0.1:${PAPERCLIP_SERVER_PORT}/api/health" >/dev/null 2>&1 && health="通过"

  if pid="$(paperclip_fallback_pid)"; then
    echo "fundeploy 托管: 运行中（PID ${pid}）"
  elif [[ -f "${PAPERCLIP_PID_FILE}" ]]; then
    echo "fundeploy 托管: 未运行（PID 文件 ${PAPERCLIP_PID_FILE} 已失效）"
  else
    echo "fundeploy 托管: 未运行"
  fi
  echo "版本: ${version:-未知}"
  echo "端口 ${PAPERCLIP_SERVER_PORT}: $(paperclip_port_open "${PAPERCLIP_SERVER_PORT}" && echo 监听中 || echo 未监听)"
  echo "健康检查 http://127.0.0.1:${PAPERCLIP_SERVER_PORT}/api/health: ${health}"
  echo "日志: ${PAPERCLIP_LOG_FILE}"
}

# 官方 service logs 读 journalctl / launchd 日志，对 fundeploy 托管的进程无效，
# 这里改 tail 它自己那份。只认 -f / -n，其余参数提示后忽略。
paperclip_logs_fallback() {
  [[ -f "${PAPERCLIP_LOG_FILE}" ]] ||
    die "没有 fundeploy 托管的日志（${PAPERCLIP_LOG_FILE} 不存在）；该实例可能未用 $0 start 启动"
  local lines="${PAPERCLIP_LOG_TAIL_LINES}" follow=0 expect_lines=0 arg
  for arg in "$@"; do
    if ((expect_lines)); then
      lines="$arg"
      expect_lines=0
      continue
    fi
    case "$arg" in
      -f | --follow) follow=1 ;;
      -n | --lines) expect_lines=1 ;;
      -n*) lines="${arg#-n}" ;;
      --lines=*) lines="${arg#--lines=}" ;;
      *) echo "提示: fundeploy 托管日志走 tail，忽略不支持的参数 ${arg}" >&2 ;;
    esac
  done
  local -a tail_args=(-n "$lines")
  ((follow == 0)) || tail_args+=(-f)
  exec tail "${tail_args[@]}" "${PAPERCLIP_LOG_FILE}"
}

# ------------------------------------------------------------------- npm 取回环境
#
# 官方 managed install 把 registry 钉死在 registry.npmjs.org：它另写一份只含 registry
# 两行的临时 .npmrc、用 npm_config_userconfig 强制 npm 子进程改用它，同时在命令行再传
# 一次 --registry / --@paperclipai:registry（cli/src/commands/install.ts 的
# PUBLIC_NPM_REGISTRY）。这是刻意的防 registry 投毒设计，有上游测试守着，因此：
#   - ~/.npmrc 里的 registry、proxy、fetch-* 对它一概无效（文件被整体替换）；
#   - npm_config_registry 环境变量也压不过它（命令行优先级最高）。
# 唯一能透进去的是环境变量：它以 env: { ...process.env, npm_config_userconfig } 启动
# npm。所以这里只设它没有在命令行显式传入的那些键 —— 取回重试/超时和代理。
#
# 另外最外层那次 `npx paperclipai@latest` 仍然会读 ~/.npmrc，并把读到的配置以
# npm_config_* 注入子进程环境。已知 npm bug（npm/cli#9783、#9912、#9968）会把这样继承
# 来的 allow-scripts 误判成显式传入的 --allow-scripts 并直接报 EALLOWSCRIPTS，而只
# unset 环境变量不够（配置本就来自文件）。所以这里照官方 install.sh 的做法，自己也写
# 一份最小 npmrc 顶掉 ~/.npmrc。刻意只在 install / upgrade / plugin install 期间生效，
# 不在脚本顶层无条件执行：status / logs / run 那些路径用 exec 收尾，EXIT trap 不触发，
# 临时文件会留在 /tmp 里。
PAPERCLIP_NPMRC_FILE=""

paperclip_npmrc_cleanup() {
  [[ -n "${PAPERCLIP_NPMRC_FILE}" ]] || return 0
  rm -f "${PAPERCLIP_NPMRC_FILE}"
  PAPERCLIP_NPMRC_FILE=""
}

paperclip_prepare_npm_env() {
  unset npm_config_allow_scripts
  if [[ -z "${PAPERCLIP_NPMRC_FILE}" ]] && command -v npm >/dev/null 2>&1; then
    if PAPERCLIP_NPMRC_FILE="$(mktemp "${TMPDIR:-/tmp}/fundeploy-paperclip-npmrc.XXXXXX" 2>/dev/null)"; then
      local registry="${PAPERCLIP_NPM_REGISTRY%/}"
      printf 'registry=%s/\n@paperclipai:registry=%s/\n' "$registry" "$registry" >"${PAPERCLIP_NPMRC_FILE}"
      export npm_config_userconfig="${PAPERCLIP_NPMRC_FILE}"
      trap paperclip_npmrc_cleanup EXIT
    else
      PAPERCLIP_NPMRC_FILE=""
    fi
  fi

  export npm_config_fetch_retries="${PAPERCLIP_NPM_FETCH_RETRIES}"
  export npm_config_fetch_retry_mintimeout="${PAPERCLIP_NPM_FETCH_RETRY_MINTIMEOUT}"
  export npm_config_fetch_retry_maxtimeout="${PAPERCLIP_NPM_FETCH_RETRY_MAXTIMEOUT}"
  export npm_config_fetch_timeout="${PAPERCLIP_NPM_FETCH_TIMEOUT}"
  if [[ "${PAPERCLIP_NPM_PREFER_OFFLINE:-0}" == "1" ]]; then
    export npm_config_prefer_offline=true
  fi

  local proxy="${PAPERCLIP_HTTPS_PROXY}"
  [[ -n "$proxy" ]] || return 0
  # 代理只做隧道（HTTPS 经 CONNECT 透传，TLS 仍是端到端），因此本机 http:// 代理是
  # 正常用法，这里不像 GitHub 下载改写那样强制 https://。但仍限定已知 scheme，避免把
  # registry 地址或 shell 片段误填进来后静默生效。
  case "$proxy" in
    http://* | https://* | socks5://* | socks5h://*) ;;
    *)
      echo "[fundeploy paperclip] 忽略 PAPERCLIP_HTTPS_PROXY（${proxy}）：仅支持 http:// https:// socks5:// socks5h://" >&2
      return 0
      ;;
  esac
  echo "[fundeploy paperclip] npm 下载走代理 ${proxy}" >&2
  export npm_config_proxy="$proxy" npm_config_https_proxy="$proxy"
  export HTTP_PROXY="$proxy" HTTPS_PROXY="$proxy" http_proxy="$proxy" https_proxy="$proxy"
  if [[ -n "${PAPERCLIP_NO_PROXY}" ]]; then
    export NO_PROXY="${PAPERCLIP_NO_PROXY}" no_proxy="${PAPERCLIP_NO_PROXY}"
  fi
  return 0
}

paperclip_print_network_hint() {
  [[ "${PAPERCLIP_NETWORK_HINT:-1}" == "0" || -n "${PAPERCLIP_HTTPS_PROXY}" ]] && return 0
  cat >&2 <<'HINT'
提示: Paperclip 的依赖树实测约 1.4G（@openai/codex 370M、@paperclipai/server 360M、
      @anthropic-ai/* 220M）。官方 managed install 把 registry 钉死在 npmjs.org，换
      镜像对它无效（刻意的防投毒设计，有上游测试守着），受限网络请走代理：
      PAPERCLIP_HTTPS_PROXY=http://127.0.0.1:7890。PAPERCLIP_NETWORK_HINT=0 隐藏本行。
HINT
}

# --------------------------------------------------------------------- 安装

cmd_install() {
  paperclip_prepare_npm_env
  paperclip_print_network_hint

  # 执行器优先级：本机已装的 CLI > npx。
  # npx paperclipai@latest 会把整棵依赖树再下一份到 ~/.npm/_npx（实测 1.4G），而它唯一
  # 的用途是执行一次 install 子命令。install 自己会解析目标版本，与执行它的 CLI 版本
  # 无关，所以复用已有 CLI 不会装到旧版。PAPERCLIP_INSTALL_VIA_NPX=1 是本机 CLI 损坏
  # 时的逃生门。
  local executable
  if [[ "${PAPERCLIP_INSTALL_VIA_NPX:-0}" != "1" ]] &&
    executable="$(paperclip_executable)" && "$executable" --version >/dev/null 2>&1; then
    echo "==> 复用本机 paperclipai 执行 managed install（跳过 npx 的重复下载）" >&2
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" \
      "$executable" install --yes "$@"
  else
    # 只有这条路需要版本门槛：此时还没有 shim，npx 用的就是 PATH 上那个 node。
    require_node
    command -v npx >/dev/null 2>&1 || die "未找到 npx（Node.js 自带）"
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" paperclipai@latest install --yes "$@"
  fi
  paperclip_cli --version
}

# ------------------------------------------------------------------ 版本与升级

paperclip_channel_option() {
  case "${1:-canary}" in
    canary) printf '%s\n' --canary ;;
    prod | latest | stable) printf '%s\n' --latest ;;
    *) die "未知更新渠道: ${1}（支持 canary / prod）" ;;
  esac
}

# 依次输出四行 currentVersion / targetVersion / updateAvailable / downgrade；
# 拿不到可解析的 JSON 时返回非 0（离线、或 CLI 还没装）。
# 同样刻意分行而非 \t 拼接，理由见 paperclip_service_state。
#
# 刻意不看 paperclip_cli 的返回值：--check 把「有可用更新」用退出码 10 表示（像 diff
# 那样的信号式退出码，payload 里 `if (options.check) process.exitCode = 10`），当成失败
# 就恰好把唯一需要升级的那种情况判成「查不到版本」。判据只有一个：stdout 能不能解析。
paperclip_update_check() {
  local json
  json="$(PAPERCLIP_CLI_NO_DIAGNOSE=1 paperclip_cli update "$1" --check --json 2>/dev/null)" || true
  [[ -n "$json" ]] || return 1
  printf '%s' "$json" | node -e '
    const fs = require("node:fs");
    let value;
    try { value = JSON.parse(fs.readFileSync(0, "utf8")); } catch { process.exit(1); }
    if (!value || typeof value !== "object") process.exit(1);
    console.log(String(value.currentVersion || ""));
    console.log(String(value.targetVersion || ""));
    console.log(value.updateAvailable === true ? "true" : "false");
    console.log(value.downgrade === true ? "true" : "false");
  ' 2>/dev/null || return 1
}

paperclip_read_update_check() {
  PAPERCLIP_UPDATE_CURRENT=""
  PAPERCLIP_UPDATE_TARGET=""
  PAPERCLIP_UPDATE_AVAILABLE=""
  PAPERCLIP_UPDATE_DOWNGRADE=""
  local state
  state="$(paperclip_update_check "$1")" || return 1
  {
    read -r PAPERCLIP_UPDATE_CURRENT || true
    read -r PAPERCLIP_UPDATE_TARGET || true
    read -r PAPERCLIP_UPDATE_AVAILABLE || true
    read -r PAPERCLIP_UPDATE_DOWNGRADE || true
  } <<<"$state"
}

paperclip_wait_for_update_health() {
  paperclip_load_config
  if paperclip_wait_for_health "${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC}"; then
    echo "Paperclip 升级完成，健康检查通过。"
    return 0
  fi
  # 此时 payload 已替换且 start 已执行过，服务很可能只是还在启动 —— 先把状态
  # 打出来，避免把「仍在启动」误读成「升级失败」。
  echo "==> 健康检查未通过，当前服务状态：" >&2
  cmd_status || true
  die "Paperclip 升级后未在 ${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC} 秒内通过健康检查（payload 已更新，服务可能仍在启动，可用 $0 status / logs 确认）"
}

# 停服 → 一次官方 update（参数由调用方给）→ doctor --repair → 重启 → 健康检查。
# upgrade 和 rollback 共用这套编排。
#
# 刻意不做升级前备份，也给官方 update 传 --no-backup 关掉它自己那次：实测一次备份 370M、
# 耗时数十秒，而 update 换的是 payload，不会动 ${PAPERCLIP_DATABASE_DIR} 里的数据，payload
# 要退回去有 rollback。备份是单独的 cmd_backup，需要就先执行一次再升级。
paperclip_apply_update() {
  local -a update_args=("$@")

  paperclip_load_config
  # 两条通道都要停：官方 service stop 在无服务管理器的机器上会「假成功」（返回 0 且
  # 什么都没停），fundeploy 托管的那个进程只能从 PID 文件收。刻意不走 cmd_stop ——
  # 它在 PAPERCLIP_FALLBACK=off 时会 die，而这里要的是尽力而为，真正的护栏是紧随
  # 其后的端口检查。
  paperclip_cli service stop || true
  paperclip_stop_fallback --quiet
  # 「假成功」会让上面两步都没停下任何东西，继续下去就是在服务运行中替换 payload；
  # 用 run 前台启动的实例正属于这种情况。因此无条件确认端口已释放。
  if paperclip_port_open "${PAPERCLIP_SERVER_PORT}"; then
    die "Paperclip 仍在监听 ${PAPERCLIP_SERVER_PORT}（可能由 $0 run 前台启动，不受服务管理器管辖），请先停止后重试"
  fi
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || paperclip_stop_embedded_postgres

  if ! paperclip_cli update "${update_args[@]}" --no-backup; then
    # 放在子 shell 里尽力拉起来：cmd_start 失败时走 die（即 exit），`|| true` 拦不住，
    # 会把 update 的失败吞成整脚本退出。这里要保留 update 本身的失败语义。
    (cmd_start) || true
    return 1
  fi

  # 取代此前三段 macOS arm64 专属的 embedded-postgres 补丁（pnpm allowBuilds、
  # hydrate-symlinks、删 launchd 里的 DATABASE_URL）：上游现在有专门的修复命令，会检查
  # 配置、install store、manifest、current 链接、shim、PATH、目录权限等并尝试修好。
  # 刻意放在 start 之前 —— 官方 `paperclipai run` 自己也是 onboard + doctor --repair
  # 之后才启动。不让它的返回值挡住启动：服务停着时「数据库连不上」这类检查会判失败，
  # 而真正的闸门是启动后的健康检查。
  paperclip_cli doctor --repair --yes ||
    echo "提示: doctor --repair 未全部通过，继续启动；可在启动后执行 $0 doctor 看详情" >&2

  cmd_start
  paperclip_wait_for_update_health
}

cmd_check() {
  local option
  option="$(paperclip_channel_option "${1:-canary}")"
  if ! paperclip_read_update_check "$option"; then
    die "无法取得 ${1:-canary} 通道的版本信息（网络不可达，或还没装 paperclipai）"
  fi
  echo "当前版本: ${PAPERCLIP_UPDATE_CURRENT:-未知}"
  echo "通道目标: ${PAPERCLIP_UPDATE_TARGET:-未知}"
  if [[ "${PAPERCLIP_UPDATE_DOWNGRADE}" == "true" ]]; then
    echo "结论: 目标版本比当前版本旧，执行 upgrade 会是一次降级（需确认）"
  elif [[ "${PAPERCLIP_UPDATE_AVAILABLE}" == "true" ]]; then
    echo "结论: 有新版本可升级，执行: $0 upgrade ${1:-canary}"
  else
    echo "结论: 已是该通道最新版"
  fi
}

cmd_upgrade() {
  local channel="${1:-canary}" option
  option="$(paperclip_channel_option "$channel")"
  paperclip_prepare_npm_env
  paperclip_print_network_hint

  local -a update_args=("$option")
  # 先问官方要不要动，别自己比版本号。拿不到结果（离线、首次安装）时不阻塞，照常往下走。
  if paperclip_read_update_check "$option"; then
    if [[ "${PAPERCLIP_UPDATE_DOWNGRADE}" == "true" ]]; then
      # canary 机器上 --latest 指向的 stable 往往比当前版本旧；官方 update 把这种情况
      # 当显式降级、要求 -y 确认，不补的话非交互环境里会卡住。
      echo "==> ${channel} 通道的 ${PAPERCLIP_UPDATE_TARGET} 比当前 ${PAPERCLIP_UPDATE_CURRENT} 旧，这是一次降级。" >&2
      fundeploy_confirm_destructive \
        "确认把 Paperclip 从 ${PAPERCLIP_UPDATE_CURRENT} 降级到 ${PAPERCLIP_UPDATE_TARGET}？（不备份数据库，需要先 $0 backup）" \
        PAPERCLIP_UPGRADE_YES || return 1
      update_args+=(-y)
    elif [[ "${PAPERCLIP_UPDATE_AVAILABLE}" != "true" ]]; then
      echo "==> 已是 ${channel} 通道最新版 ${PAPERCLIP_UPDATE_CURRENT}，跳过更新（未重启服务；需重启用: $0 restart）" >&2
      return 0
    else
      echo "==> ${channel} 通道有新版本: ${PAPERCLIP_UPDATE_CURRENT} → ${PAPERCLIP_UPDATE_TARGET}" >&2
    fi
  fi

  paperclip_apply_update "${update_args[@]}"
}

cmd_rollback() {
  paperclip_prepare_npm_env
  paperclip_apply_update --rollback
}

# 独立的备份命令。upgrade 以前每次都自动备一次（本机 370M、耗时数十秒），现在不再自动做，
# 需要就显式执行这个。embedded PostgreSQL 是服务的子进程，服务停着它也没在跑、db:backup
# 必然连不上——在这里先说清楚，而不是把上游的连接错误丢给用户自己猜。
cmd_backup() {
  paperclip_load_config
  if [[ "${PAPERCLIP_DATABASE_MODE}" == "embedded-postgres" ]] && ! paperclip_port_open "${PAPERCLIP_DATABASE_PORT}"; then
    die "embedded PostgreSQL 未在监听 ${PAPERCLIP_DATABASE_PORT}（它随 Paperclip 一起停），请先 $0 start 再备份"
  fi
  paperclip_cli db:backup "$@"
}

cmd_channels() { paperclip_cli channels "$@"; }
cmd_doctor() { paperclip_cli doctor "$@"; }
cmd_health() { paperclip_cli health "$@"; }

# ------------------------------------------------------------------ 配置与插件

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
  paperclip_prepare_npm_env
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

# ------------------------------------------------------------------- 生命周期

# start / restart 内部的 ensureCurrent() 会写 ~/.config/systemd/user 单元并
# daemon-reload，所以单元文件本身能自愈；但 systemctl --user enable（开机自启）和
# loginctl enable-linger 只有官方 service install 会做，此前脚本没有暴露它，于是
# fundeploy 管理的 Paperclip 不会开机自启。
cmd_service_install() {
  paperclip_cli service install "$@"
}

cmd_start() {
  paperclip_read_service_state
  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" != "false" ]]; then
    # supervisor 可用（含状态拿不到的情况）时维持官方路径：systemd 的 Type=notify /
    # launchd 自己会等到就绪，不必再叠一层健康检查。
    paperclip_cli service start "$@"
    return
  fi
  paperclip_require_fallback
  paperclip_start_fallback
}

cmd_stop() {
  paperclip_read_service_state
  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" != "false" ]]; then
    paperclip_cli service stop "$@"
    # 无条件再停一次 fundeploy 托管的进程：机器从「无 supervisor」变成「有 supervisor」
    # 后，上次 start 留下的那个进程不受 supervisor 管辖，只能从 PID 文件收尾。
    paperclip_stop_fallback --quiet
    return
  fi
  # 关掉兜底时也不该假成功：官方 stop 在这种机器上什么都不会停。
  paperclip_require_fallback
  paperclip_stop_fallback
}

cmd_status() {
  paperclip_read_service_state
  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" != "false" ]]; then
    local rc=0
    paperclip_cli service status "$@" || rc=$?
    # 仅在确实留有 PID 文件时才追加，避免正常 supervisor 机器上多出无关输出。
    [[ -f "${PAPERCLIP_PID_FILE}" ]] || return "$rc"
    echo ""
  else
    echo "官方服务管理器不可用: ${PAPERCLIP_SERVICE_MESSAGE:-未检测到可用的服务管理器}"
  fi
  paperclip_status_fallback
}

cmd_logs() {
  paperclip_read_service_state
  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" != "false" ]]; then
    paperclip_cli service logs "$@"
    return
  fi
  paperclip_require_fallback
  paperclip_logs_fallback "$@"
}

cmd_restart() {
  paperclip_read_service_state

  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" == "false" ]]; then
    paperclip_require_fallback
    paperclip_stop_fallback
    paperclip_start_fallback
    return
  fi

  if [[ "${PAPERCLIP_SERVICE_SUPPORTED}" == "unknown" ]]; then
    # 状态拿不到就交给官方 restart 自己报错，不猜测。
    paperclip_cli service restart "$@"
    return
  fi

  # 官方 service restart 只做热重启：writeHotRestartIntent 第一步就是 if (!status.pid)
  # throw，所以服务没在跑时它永远失败 —— 那种场景要的其实是 start。
  if [[ -z "${PAPERCLIP_SERVICE_PID}" ]]; then
    echo "==> 服务未在运行（无 supervisor pid），改为 start" >&2
    paperclip_cli service start
    return
  fi
  paperclip_cli service restart "$@"
}

cmd_run() {
  require_node
  local executable running
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  # 官方 run 自带的「同实例已在 supervisor 下运行」检查覆盖不到 fundeploy 托管的进程
  # （那种机器上根本没有 supervisor），不挡住就会起第二个实例去抢同一个端口。
  if running="$(paperclip_fallback_pid)"; then
    die "Paperclip 已由 fundeploy 在后台运行（PID ${running}），请先执行: $0 stop"
  fi
  paperclip_load_config
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || unset DATABASE_URL
  # exec 之前清掉临时 npmrc：EXIT trap 在 exec 换掉进程映像后不会触发。
  paperclip_npmrc_cleanup
  exec env PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" run "$@"
}

cmd_uninstall() {
  echo "Paperclip 数据目录 ${PAPERCLIP_HOME} 不会删除。" >&2
  fundeploy_confirm_destructive "确认卸载 Paperclip 官方服务和 CLI？" PAPERCLIP_UNINSTALL_YES || return 1

  # 先停再卸：官方 service uninstall 在无服务管理器的机器上同样「假成功」，
  # 不先收掉 fundeploy 托管的进程就会在服务运行中删掉它的 payload。
  paperclip_stop_fallback
  paperclip_cli service uninstall
  paperclip_cli uninstall
  echo "已卸载 Paperclip 服务和 CLI；数据保留在 ${PAPERCLIP_HOME}。"
}

# ---------------------------------------------------------------------- 入口

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install | install-prod) cmd_install "$@" ;;
    install-canary) cmd_install --canary "$@" ;;
    check) cmd_check "$@" ;;
    update | upgrade) cmd_upgrade "$@" ;;
    rollback) cmd_rollback "$@" ;;
    backup) cmd_backup "$@" ;;
    channels) cmd_channels "$@" ;;
    doctor) cmd_doctor "$@" ;;
    health) cmd_health "$@" ;;
    service-install) cmd_service_install "$@" ;;
    onboard) cmd_onboard "$@" ;;
    plugin) cmd_plugin "$@" ;;
    run) cmd_run "$@" ;;
    start) cmd_start "$@" ;;
    stop) cmd_stop "$@" ;;
    restart) cmd_restart "$@" ;;
    status) cmd_status "$@" ;;
    logs) cmd_logs "$@" ;;
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
      "install-canary  安装开发版（官方 managed install）" \
      "install-prod    安装正式版（官方 managed install）" \
      "check           只查有没有新版，不改动任何东西" \
      "upgrade         停服 → 官方 update → 修复 → 重启（不备份）" \
      "rollback        回退到保留的上一份 payload" \
      "backup          备份数据库（embedded 模式需服务在跑）" \
      "channels        查看发布通道与当前跟随的通道" \
      "doctor          官方诊断（只读）" \
      "doctor-repair   官方诊断并尝试自动修复" \
      "health          官方健康检查" \
      "service-install 注册后台服务并设置开机自启" \
      "onboard         首次配置向导" \
      "plugin-list     列出 awesome-paperclip 收录的插件" \
      "plugin-install  从清单里选一个插件安装" \
      "plugin-installed 列出已安装的插件" \
      "start           后台启动并等健康检查通过" \
      "run             前台运行" \
      "stop            停止" \
      "restart         热重启" \
      "status          查看 supervisor、版本与健康状态" \
      "logs            查看日志（跟随输出）" \
      "uninstall       卸载服务和 CLI，保留数据" \
      "help            命令帮助" \
      "quit            返回")" || break
    [[ -n "$pick" ]] || break
    pick="${pick%% *}"
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      doctor-repair) ( dispatch doctor --repair ) ;;
      plugin-list) ( dispatch plugin list ) ;;
      plugin-install) ( dispatch plugin install ) ;;
      plugin-installed) ( dispatch plugin installed ) ;;
      logs) ( dispatch logs -f ) ;;
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
