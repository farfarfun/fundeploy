#!/usr/bin/env bash
# Paperclip (https://github.com/paperclipai/paperclip) 本机服务。
# CLI 安装、升级和后台服务均交给 Paperclip 官方命令管理。

set -euo pipefail

# 已知 npm bug（npm/cli#9783、#9912、#9968）：用户 .npmrc 中的 allow-scripts
# 配置会被 npm 读入后传给内部再次触发的 npm install（例如 paperclipai 安装器
# 自身的依赖安装），而该内部安装是 project-scoped 的，npm 会把继承来的
# allow-scripts 策略误判为显式传入 --allow-scripts 并直接报错 EALLOWSCRIPTS。
# 仅 unset 环境变量并不够（该配置本就来自 .npmrc 文件而非环境变量），这里生成
# 一份去掉 allow-scripts 的用户配置副本，并通过 npm_config_userconfig 让所有
# 子进程改用它。
unset npm_config_allow_scripts
if command -v npm >/dev/null 2>&1; then
  _paperclip_npm_userconfig="$(npm config get userconfig 2>/dev/null || true)"
  if [[ -n "${_paperclip_npm_userconfig}" && -f "${_paperclip_npm_userconfig}" ]] &&
    grep -Eq '^[[:space:]]*allow-scripts[[:space:]]*=' "${_paperclip_npm_userconfig}" 2>/dev/null; then
    _paperclip_sanitized_npmrc="$(mktemp "${TMPDIR:-/tmp}/fundeploy-paperclip-npmrc.XXXXXX")"
    grep -Ev '^[[:space:]]*allow-scripts[[:space:]]*=' "${_paperclip_npm_userconfig}" >"${_paperclip_sanitized_npmrc}"
    export npm_config_userconfig="${_paperclip_sanitized_npmrc}"
    trap 'rm -f "${_paperclip_sanitized_npmrc}"' EXIT
  fi
  unset _paperclip_npm_userconfig
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

PAPERCLIP_HOME="${PAPERCLIP_HOME:-${HOME}/.paperclip}"
PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID:-default}"
# 官方源。两个用途：npx 回落那一步的 registry，以及预装产物的 integrity 校验基准
# （见 paperclip_verify_payload_scope）。除非官方换域名，不要改。
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
# 整棵依赖树的预装源（见 paperclip_prefetch_payload）。留空则沿用本机 npm 自己的
# registry（npm config get registry）；设为 off 关闭预装，严格走官方路径。
PAPERCLIP_NPM_MIRROR="${PAPERCLIP_NPM_MIRROR:-}"
PAPERCLIP_NPM_PREFETCH="${PAPERCLIP_NPM_PREFETCH:-auto}"
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

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

命令:
  install          官方 managed install（正式版）
  install-canary   官方 managed install（开发版）
  install-prod     install 的别名
  upgrade [canary|prod] 官方升级、备份、迁移及服务重启（默认 canary；别名 update）
  service-install [--enable-linger] [--no-start-on-login]
                   注册后台服务并设置开机自启；无登录会话的机器需 --enable-linger
  onboard          首次配置；NONINTERACTIVE=1 时默认追加 --yes
  plugin list                  列出 awesome-paperclip 收录的插件
  plugin install [npm-package] 安装插件；不指定包名时从清单选择
  plugin installed             列出当前已安装的插件
  start             启动官方后台服务
  run               使用 paperclipai run 前台启动
  stop              停止官方后台服务
  restart           热重启官方后台服务；未在运行时自动改为 start
  status            查看官方 supervisor 与健康状态
  logs [options]    查看官方服务日志，例如 logs -f
  uninstall         卸载官方服务和 CLI，保留 ${PAPERCLIP_HOME} 数据

说明:
  - Linux 使用 systemd --user，macOS 使用 LaunchAgent。
  - systemd --user 需要一个用户级 systemd 实例；容器、WSL1 或没有登录会话的
    终端里不存在，官方 CLI 会提示改用 run。此时先在有登录会话的 shell 执行
    service-install --enable-linger（等价于 loginctl enable-linger）。
  - 配置由 paperclipai onboard / configure 管理。
  - PAPERCLIP_INSTANCE_ID 默认为 ${PAPERCLIP_INSTANCE_ID}。
  - upgrade 完成后检查 PostgreSQL、Paperclip 端口及 /api/health。

受限网络（install / upgrade 下载约 1.4G）:
  - PAPERCLIP_NPM_MIRROR      整棵依赖树的预装源，如 https://registry.npmmirror.com；
                              留空则沿用本机 npm 自己的 registry
  - PAPERCLIP_NPM_PREFETCH=off  关闭预装，严格走官方直连 npmjs.org 的路径
  - PAPERCLIP_HTTPS_PROXY     npm 代理，如 http://127.0.0.1:7890；留空为直连
  - PAPERCLIP_NO_PROXY        配合上一项的排除列表
  - PAPERCLIP_NETWORK_HINT=0  隐藏直连提示
  - PAPERCLIP_INSTALL_VIA_NPX=1  强制 install 走 npx（默认复用已有 CLI 以省一份下载）
  - PAPERCLIP_NPM_PREFER_OFFLINE=1  优先用 npm cache，减少重装同版本时的网络往返
  - PAPERCLIP_NPM_FETCH_RETRIES / _FETCH_TIMEOUT 等可覆盖 npm 取回重试与超时

  官方 managed install 把 registry 钉死在 npmjs.org，改 npm 配置对它无效。本脚本改为
  先用可达的源把整棵树装到官方的 payload 路径，再让官方 install 走它的「payload 已存在
  即复用」分支（实测几秒、零下载）。预装后会用 npmjs.org 的 integrity 逐个核对
  @paperclipai/*，不一致就丢弃并回落官方直连路径。详见 paperclip_prefetch_payload。
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
  if ((rc != 0)) && ! "$executable" --version >/dev/null 2>&1; then
    echo "提示: ${executable} 无法运行，paperclipai 安装可能已损坏，可执行: $0 install" >&2
  fi
  return "$rc"
}

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

# paperclipai 的 managed install 把 registry 硬编码为 registry.npmjs.org，并且会另写
# 一份只含 registry 两行的临时 .npmrc、用 npm_config_userconfig 强制 npm 子进程改用它，
# 同时在命令行再传一次 --registry / --@paperclipai:registry（cli/src/commands/install.ts
# 的 PUBLIC_NPM_REGISTRY）。这是刻意的防 registry 投毒设计，有测试守着，因此：
#   - ~/.npmrc 里的 registry、proxy、fetch-* 对它一概无效（文件被整体替换）；
#   - npm_config_registry 环境变量也压不过它（命令行优先级最高）。
# 唯一能透进去的是环境变量：它以 env: { ...process.env, npm_config_userconfig } 启动
# npm。所以这里只设它没有在命令行显式传入的那些键 —— 取回重试/超时和代理 —— 不去和
# 它的 --registry 对抗。
paperclip_apply_npm_network_env() {
  export npm_config_fetch_retries="${PAPERCLIP_NPM_FETCH_RETRIES}"
  export npm_config_fetch_retry_mintimeout="${PAPERCLIP_NPM_FETCH_RETRY_MINTIMEOUT}"
  export npm_config_fetch_retry_maxtimeout="${PAPERCLIP_NPM_FETCH_RETRY_MAXTIMEOUT}"
  export npm_config_fetch_timeout="${PAPERCLIP_NPM_FETCH_TIMEOUT}"
  if [[ "${PAPERCLIP_NPM_PREFER_OFFLINE:-0}" == "1" ]]; then
    export npm_config_prefer_offline=true
  fi

  local proxy="${PAPERCLIP_HTTPS_PROXY}"
  if [[ -z "$proxy" ]]; then
    return 0
  fi
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
  if [[ "${PAPERCLIP_NETWORK_HINT:-1}" == "0" || -n "${PAPERCLIP_HTTPS_PROXY}" ]]; then
    return 0
  fi
  paperclip_prefetch_registry >/dev/null 2>&1 && return 0
  cat >&2 <<'HINT'
提示: Paperclip 的依赖树实测约 1.4G（@openai/codex 370M、@paperclipai/server 360M、
      @anthropic-ai/* 220M），而 managed install 固定直连 registry.npmjs.org。
      受限网络可设 PAPERCLIP_NPM_MIRROR=<https 镜像> 让整棵树改从该源预装，
      或设 PAPERCLIP_HTTPS_PROXY=http://127.0.0.1:7890 之类的本机代理。
      PAPERCLIP_NETWORK_HINT=0 可隐藏本行。
HINT
}

# ===== 整树预装（受限网络的主要加速手段）=====
#
# 上游把 registry 钉死在 npmjs.org 且无法从外部覆盖（见 paperclip_apply_npm_network_env），
# 所以无法直接让它改走镜像。但 install-store 的 installNpmPayload() 在 payload 目录
# 已存在时，只跑一次冒烟检查就复用，完全跳过 npm install：
#
#   const payloadPath = payloadPathFor(paths, "npm", version);
#   if (fs.existsSync(payloadPath)) { await smokePayload(...); return { reused: true }; }
#
# 实测该分支 4.5 秒、零下载（输出 "Activated cached paperclipai ..."）。于是这里先用
# 可达的源把整棵树装到正式 payload 路径，再把 manifest / current 链接 / shim 的写入交回
# 官方 install。好处是整树都走镜像 —— 包括官方额外钉死的 @paperclipai 那 434M，而只改
# @scope:registry 环境变量的做法覆盖不到它。
#
# 代价：依赖的 integrity 来自镜像的 packument 而非 npmjs.org，相当于把上游刻意的防投毒
# 决策换成了对镜像的信任。因此预装后强制用 npmjs.org 的单版本端点逐个核对 @paperclipai/*
# 的 integrity（该 scope 承载 CLI 与 server 本体，被替换等价于任意代码执行），不一致就
# 丢弃 staging、回落官方直连路径。实测 registry.npmmirror.com 与 npmjs.org 字节一致。

# 输出预装用的 registry；未启用或无可用源时返回 1。
paperclip_prefetch_registry() {
  [[ "${PAPERCLIP_NPM_PREFETCH}" == "off" ]] && return 1
  local url="${PAPERCLIP_NPM_MIRROR}"
  if [[ -z "$url" ]]; then
    command -v npm >/dev/null 2>&1 || return 1
    url="$(npm config get registry 2>/dev/null || true)"
  fi
  url="${url%/}"
  [[ -n "$url" && "$url" != "null" && "$url" != "undefined" ]] || return 1
  # 与 fundeploy-github-download.sh 的约定一致：只接受 https://。预装源能决定装进
  # payload 的每一个字节，不允许降级到明文 http://。
  if [[ "$url" != https://* ]]; then
    echo "[fundeploy paperclip] 忽略非 https:// 的预装源（${url}）" >&2
    return 1
  fi
  # 已经是官方源时预装没有意义：官方 install 自己就走它，且能命中同一份 npm cache。
  [[ "$url" == "${PAPERCLIP_NPM_REGISTRY%/}" ]] && return 1
  printf '%s\n' "$url"
}

# 从 npmjs.org 的单版本端点解析 tag 或精确版本，输出 "版本<TAB>integrity"。
# 用单版本端点而不是完整 packument：paperclipai 已有 1700+ 个版本，完整 packument 很大，
# 而 /paperclipai/canary 这种响应实测仅约 3KB。
paperclip_npm_official_manifest() {
  local spec="$1"
  command -v curl >/dev/null 2>&1 || return 1
  _fundeploy_github_download_curl -fsSL --max-time 60 \
    "${PAPERCLIP_NPM_REGISTRY%/}/paperclipai/${spec}" 2>/dev/null | node -e '
    let body = "";
    process.stdin.on("data", (chunk) => (body += chunk)).on("end", () => {
      try {
        const manifest = JSON.parse(body);
        if (!manifest.version || !manifest.dist || !manifest.dist.integrity) process.exit(1);
        console.log(manifest.version + "\t" + manifest.dist.integrity);
      } catch {
        process.exit(1);
      }
    });
  ' 2>/dev/null
}

# 从 install 的参数推出要预装的 npm spec；git-ref 安装不走 npm payload，返回 1。
paperclip_prefetch_spec() {
  local spec="latest" expect_version=0 arg
  for arg in "$@"; do
    if ((expect_version)); then
      spec="$arg"
      expect_version=0
      continue
    fi
    case "$arg" in
      --ref | --ref=* | --repo | --repo=*) return 1 ;;
      --canary) spec="canary" ;;
      --version) expect_version=1 ;;
      --version=*) spec="${arg#--version=}" ;;
    esac
  done
  printf '%s\n' "$spec"
}

# 预检目标版本的 @paperclipai/* 依赖在预装源上是否取得到；缺失时输出清单并返回 1。
#
# 必须预检，因为常见镜像恰好缺的就是这个 scope：registry.npmmirror.com 永久没有
# @paperclipai/server（解包 276M，超过 cnpm 的 256M 上限，其同步日志为
# "too many large versions ... maximum unpacked size: 268435456"，见 cnpm/unpkg-white-list），
# 而它是必需依赖。没有预检就会白跑一次整树 npm install —— npm 要把依赖图解析完才报
# ETARGET。这里只要两三个小请求。
#
# 只检查这个 scope：实测 @openai/codex（370M）和 @anthropic-ai/claude-agent-sdk-linux-x64
# 在 npmmirror 上都有，缺口集中在 @paperclipai，它也正是上游在命令行额外钉死的那一个。
paperclip_prefetch_source_check() {
  local registry="$1" version="$2"
  node -e '
    const official = process.argv[1].replace(/\/$/, "");
    const mirror = process.argv[2].replace(/\/$/, "");
    const version = process.argv[3];
    const get = (url) => fetch(url, { signal: AbortSignal.timeout(30000) });
    (async () => {
      let manifest;
      try {
        const response = await get(`${official}/paperclipai/${version}`);
        if (!response.ok) throw new Error("HTTP " + response.status);
        manifest = await response.json();
      } catch (err) {
        console.error("  无法从官方源读取依赖清单: " + err.message);
        process.exit(1);
      }
      const deps = Object.entries(manifest.dependencies || {}).filter(
        ([name]) => name === "paperclipai" || name.startsWith("@paperclipai/"),
      );
      const missing = [];
      await Promise.all(
        deps.map(async ([name, range]) => {
          const exact = String(range).replace(/^[\^~=v]+/, "");
          try {
            const response = await get(`${mirror}/${name}/${exact}`);
            if (!response.ok) missing.push(`${name}@${exact}（HTTP ${response.status}）`);
          } catch (err) {
            missing.push(`${name}@${exact}（${err.message}）`);
          }
        }),
      );
      if (missing.length) {
        for (const item of missing) console.error("  缺: " + item);
        process.exit(1);
      }
      console.log(String(deps.length));
    })();
  ' "${PAPERCLIP_NPM_REGISTRY}" "$registry" "$version" 2>&1
}

# 用 npmjs.org 的 integrity 核对预装产物里的 @paperclipai/*；成功时输出核对过的包数。
paperclip_verify_payload_scope() {
  local lockfile="$1"
  [[ -f "$lockfile" ]] || return 1
  node -e '
    const fs = require("node:fs");
    const registry = process.argv[2].replace(/\/$/, "");
    let lock;
    try {
      lock = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    } catch (err) {
      console.error("  无法读取 lockfile: " + err.message);
      process.exit(1);
    }
    const targets = [];
    for (const [location, info] of Object.entries(lock.packages || {})) {
      const name = location.replace(/^.*node_modules\//, "");
      if (name !== "paperclipai" && !name.startsWith("@paperclipai/")) continue;
      if (!info.version || !info.integrity) continue;
      targets.push({ name, version: info.version, integrity: info.integrity });
    }
    if (!targets.length) {
      console.error("  lockfile 中没有 @paperclipai 包，无法核对");
      process.exit(1);
    }
    (async () => {
      const problems = [];
      await Promise.all(
        targets.map(async (target) => {
          const url = `${registry}/${target.name}/${target.version}`;
          let official;
          try {
            const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
            if (!response.ok) throw new Error("HTTP " + response.status);
            official = (await response.json()).dist?.integrity;
          } catch (err) {
            problems.push(`${target.name}@${target.version}: 无法向 npmjs.org 核对（${err.message}）`);
            return;
          }
          if (official !== target.integrity) {
            problems.push(`${target.name}@${target.version}: 镜像 ${target.integrity} != npmjs.org ${official}`);
          }
        }),
      );
      if (problems.length) {
        for (const problem of problems) console.error("  " + problem);
        process.exit(1);
      }
      console.log(String(targets.length));
    })();
  ' "$lockfile" "${PAPERCLIP_NPM_REGISTRY}" 2>&1
}

# 把整棵树装进 staging 并自检；失败时由调用方清理 staging。
paperclip_prefetch_into_staging() {
  local registry="$1" version="$2" staging="$3"
  echo "==> 从 ${registry} 预装 paperclipai@${version} 整棵依赖树（约 1.4G，首次较久）" >&2
  # 与上游 installNpmPayload 的命令保持一致，只替换 registry：同样不传 --ignore-scripts，
  # 否则原生模块的 postinstall 不会执行，冒烟检查过不去。
  npm install --prefix "$staging" "paperclipai@${version}" \
    --registry="$registry" --no-audit --no-fund >&2 || {
    echo "==> 预装失败（镜像可能缺该版本），回落官方直连路径" >&2
    return 1
  }

  # 复刻上游 smokePayload：入口必须存在，且自报版本要与目标一致。先于 integrity 核对，
  # 装坏了就没必要再发网络请求。
  local entrypoint="${staging}/node_modules/paperclipai/dist/index.js"
  [[ -f "$entrypoint" ]] || {
    echo "==> 预装产物缺少 CLI 入口 ${entrypoint}，丢弃" >&2
    return 1
  }
  local reported
  reported="$(node "$entrypoint" --version 2>/dev/null | awk 'NR==1{print $1}')"
  [[ "$reported" == "$version" ]] || {
    echo "==> 预装产物自报版本 ${reported:-未知}，与目标 ${version} 不符，丢弃" >&2
    return 1
  }

  local verified
  if ! verified="$(paperclip_verify_payload_scope "${staging}/package-lock.json")"; then
    echo "${verified}" >&2
    echo "==> @paperclipai/* 的 integrity 与 npmjs.org 不一致或无法核对，丢弃预装产物并回落官方直连路径" >&2
    return 1
  fi
  echo "==> 已用 npmjs.org 核对 ${verified} 个 @paperclipai 包的 integrity" >&2
  return 0
}

# 成功时 stdout 输出已就绪 payload 的版本号。
paperclip_prefetch_payload() {
  local registry="$1" spec="$2"
  local resolved version
  resolved="$(paperclip_npm_official_manifest "$spec")" || {
    echo "==> 无法从 ${PAPERCLIP_NPM_REGISTRY} 解析 paperclipai@${spec}，跳过预装" >&2
    return 1
  }
  version="${resolved%%$'\t'*}"
  # 对齐上游 payloadPathFor 的校验，顺带挡住解析结果里的路径穿越。
  [[ "$version" =~ ^[A-Za-z0-9._-]+$ ]] || {
    echo "==> 解析到的版本号不合法（${version}），跳过预装" >&2
    return 1
  }

  local installs_root="${PAPERCLIP_HOME}/cli/installs/npm"
  local payload="${installs_root}/${version}"
  if [[ -d "$payload" ]]; then
    echo "==> payload 已存在，官方 install 将直接复用: ${version}" >&2
    printf '%s\n' "$version"
    return 0
  fi

  command -v npm >/dev/null 2>&1 || return 1

  local checked
  if ! checked="$(paperclip_prefetch_source_check "$registry" "$version")"; then
    echo "${checked}" >&2
    cat >&2 <<HINT
==> ${registry} 取不到 paperclipai@${version} 的 @paperclipai 依赖，跳过预装、改走官方直连。
    registry.npmmirror.com 永久缺 @paperclipai/server（解包 276M，超过 cnpm 的 256M
    上限），换一个不做大小限制的源即可，例如:
      PAPERCLIP_NPM_MIRROR=https://mirrors.cloud.tencent.com/npm
HINT
    return 1
  fi
  echo "==> 预检通过：${registry} 具备 ${checked} 个 @paperclipai 依赖" >&2

  mkdir -p "$installs_root" || return 1
  # 前缀刻意不同于上游的 .<version>.tmp-<pid>：上游的 finally 只删自己那个，而带这个
  # 前缀的目录也不会被 payloadPathFor 当成某个版本的 payload。
  local staging="${installs_root}/.fundeploy-prefetch-${version}.$$"
  rm -rf "$staging"
  if ! paperclip_prefetch_into_staging "$registry" "$version" "$staging"; then
    rm -rf "$staging"
    return 1
  fi
  # rename 是原子的，所以官方 install 看到的 payload 要么不存在、要么已完整自检通过。
  mv "$staging" "$payload" || {
    rm -rf "$staging"
    return 1
  }
  echo "==> 预装完成: ${payload}" >&2
  printf '%s\n' "$version"
}

# 剔除参数里的通道选择（--canary / --version），改为精确钉到已预装的版本。
paperclip_pin_version_args() {
  local version="$1"
  shift
  local pinned=() skip_next=0 arg
  for arg in "$@"; do
    if ((skip_next)); then
      skip_next=0
      continue
    fi
    case "$arg" in
      --canary) ;;
      --version) skip_next=1 ;;
      --version=*) ;;
      *) pinned+=("$arg") ;;
    esac
  done
  pinned+=(--version "$version")
  printf '%s\n' "${pinned[@]}"
}

cmd_install() {
  require_node
  paperclip_apply_npm_network_env
  paperclip_print_network_hint

  local prefetched="" registry spec
  if registry="$(paperclip_prefetch_registry)" && spec="$(paperclip_prefetch_spec "$@")"; then
    prefetched="$(paperclip_prefetch_payload "$registry" "$spec")" || prefetched=""
  fi

  local args=("$@")
  if [[ -n "$prefetched" ]]; then
    # 预装期间上游可能又发了一版（canary 实测几小时一个）。那时 tag 已指向新版本，官方
    # install 的复用分支落空，刚预装的 1.4G 白下。所以只在 tag 仍指向预装版本时保留原
    # 通道参数，否则精确钉到预装的那一个。代价是 manifest 的 channel 记为 pinned，裸
    # `paperclipai update` 会卡在该版本 —— 本脚本的 upgrade 始终显式传 --canary/--latest，
    # 不受影响。
    local recheck=""
    recheck="$(paperclip_npm_official_manifest "$spec")" || recheck=""
    if [[ -n "$recheck" && "${recheck%%$'\t'*}" != "$prefetched" ]]; then
      echo "==> ${spec} 已更新到 ${recheck%%$'\t'*}；为复用预装 payload 改按精确版本安装 ${prefetched}（channel 记为 pinned）" >&2
      mapfile -t args < <(paperclip_pin_version_args "$prefetched" "$@")
    fi
  fi

  # 执行器优先级：预装 payload 自带的 CLI > 本机已装的 CLI > npx。
  # npx paperclipai@latest 会把全部依赖再下一份到 ~/.npm/_npx（实测 1.4G），而它唯一的
  # 用途是执行一次 install 子命令。前两条路都能省掉这一份；预装成功时连首次安装也不再
  # 需要 npx。install 子命令自己会解析目标版本，与执行它的 CLI 版本无关，所以复用已有
  # CLI 不会装到旧版。
  local via_npx="${PAPERCLIP_INSTALL_VIA_NPX:-0}"
  local entrypoint="" executable
  [[ -n "$prefetched" ]] &&
    entrypoint="${PAPERCLIP_HOME}/cli/installs/npm/${prefetched}/node_modules/paperclipai/dist/index.js"
  if [[ "$via_npx" != "1" && -n "$entrypoint" && -f "$entrypoint" ]]; then
    echo "==> 用预装 payload 自带的 CLI 执行 managed install" >&2
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" node "$entrypoint" install --yes "${args[@]}"
  elif [[ "$via_npx" != "1" ]] &&
    executable="$(paperclip_executable)" && "$executable" --version >/dev/null 2>&1; then
    echo "==> 复用本机 paperclipai 执行 managed install（跳过 npx 的重复下载）" >&2
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" "$executable" install --yes "${args[@]}"
  else
    command -v npx >/dev/null 2>&1 || die "未找到 npx（Node.js 自带）"
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" paperclipai@latest install --yes "${args[@]}"
  fi
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
  # 此时 payload 已替换且 service start 已执行过，服务很可能只是还在启动 —— 先把状态
  # 打出来，避免把「仍在启动」误读成「升级失败」。
  echo "==> 健康检查未通过，当前服务状态：" >&2
  paperclip_cli service status || true
  die "Paperclip 升级后未在 ${PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC} 秒内通过健康检查（payload 已更新，服务可能仍在启动，可用 $0 status / logs 确认）"
}

cmd_upgrade() {
  local channel="${1:-canary}" option
  case "$channel" in
    canary) option=--canary ;;
    prod) option=--latest ;;
    *) die "未知更新渠道: ${channel}（支持 canary / prod）" ;;
  esac

  local spec
  case "$channel" in
    canary) spec=canary ;;
    prod) spec=latest ;;
  esac

  paperclip_apply_npm_network_env
  paperclip_print_network_hint

  # 预装刻意放在停服之前：它要下约 1.4G，而之后 update 命中 payload 复用分支只要几秒。
  # 先装好再停服，能把停机时间从「下载整棵依赖树」压到「一次冒烟检查」。
  local prefetched="" registry
  if registry="$(paperclip_prefetch_registry)"; then
    prefetched="$(paperclip_prefetch_payload "$registry" "$spec")" || prefetched=""
  fi

  # 版本没变就不必停服再启：update 在版本相同时只会打印 already up-to-date，但停服、
  # 重启和健康检查照样走一遍，白中断一次。
  local current=""
  if [[ -n "$prefetched" ]]; then
    current="$(paperclip_cli --version 2>/dev/null | awk 'NR==1{print $1}')" || current=""
    if [[ -n "$current" && "$current" == "$prefetched" ]]; then
      echo "==> 已是 ${spec} 通道最新版 ${current}，跳过更新（未重启服务；需重启用: $0 restart）" >&2
      return 0
    fi
  fi

  paperclip_load_config
  [[ ! -f "${PAPERCLIP_CONFIG_PATH}" ]] || paperclip_cli db:backup

  paperclip_cli service stop || true
  # 原先只在 service stop 失败时才查端口，但无服务管理器时 stop 会「假成功」（返回 0
  # 且什么都没停，见 paperclip_service_state 的注释），端口检查会被整个跳过 —— 用
  # run 前台启动的实例正属于这种情况，继续下去就会在服务运行中替换 payload。
  # 因此改为无条件确认端口已释放。
  if paperclip_port_open "${PAPERCLIP_SERVER_PORT}"; then
    die "Paperclip 仍在监听 ${PAPERCLIP_SERVER_PORT}（可能由 $0 run 前台启动，不受服务管理器管辖），请先停止后重试"
  fi
  [[ "${PAPERCLIP_DATABASE_MODE}" != "embedded-postgres" ]] || paperclip_stop_embedded_postgres
  paperclip_allow_embedded_postgres_build

  # 与 cmd_install 同一个竞态：预装期间 tag 可能已经前进，那时复用分支落空、刚下的
  # 1.4G 白费，故改为精确钉到预装的版本。
  local update_args=("$option") recheck=""
  if [[ -n "$prefetched" ]]; then
    recheck="$(paperclip_npm_official_manifest "$spec")" || recheck=""
    if [[ -n "$recheck" && "${recheck%%$'\t'*}" != "$prefetched" ]]; then
      echo "==> ${spec} 已更新到 ${recheck%%$'\t'*}；为复用预装 payload 改按精确版本更新 ${prefetched}（channel 记为 pinned）" >&2
      update_args=(--version "$prefetched")
    fi
  fi

  if ! paperclip_cli update "${update_args[@]}" --no-backup; then
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
  paperclip_apply_npm_network_env
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

# start / restart 内部的 ensureCurrent() 会写 ~/.config/systemd/user 单元并
# daemon-reload，所以单元文件本身能自愈；但 systemctl --user enable（开机自启）和
# loginctl enable-linger 只有官方 service install 会做，此前脚本没有暴露它，于是
# fundeploy 管理的 Paperclip 不会开机自启。
cmd_service_install() {
  paperclip_cli service install "$@"
}

cmd_restart() {
  local state supported="" pid="" message=""
  if ! state="$(paperclip_service_state)"; then
    # 状态拿不到就交给官方 restart 自己报错，不猜测。
    paperclip_cli service restart "$@"
    return
  fi
  # read 读到最后一行若无换行会返回非 0，但变量已赋值，故逐行 || true。
  {
    read -r supported || true
    read -r pid || true
    IFS= read -r message || true
  } <<<"$state"

  if [[ "$supported" == "false" ]]; then
    die "${message:-未检测到可用的服务管理器}
可选做法: $0 run 前台启动，或在有登录会话的 shell 执行 $0 service-install --enable-linger"
  fi

  # 官方 service restart 只做热重启：writeHotRestartIntent 第一步就是 if (!status.pid)
  # throw，所以服务没在跑时它永远失败 —— 那种场景要的其实是 start。
  if [[ -z "$pid" ]]; then
    echo "==> 服务未在运行（无 supervisor pid），改为 start" >&2
    paperclip_cli service start
    return
  fi
  paperclip_cli service restart "$@"
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
    install | install-prod) cmd_install "$@" ;;
    install-canary) cmd_install --canary "$@" ;;
    update|upgrade) cmd_upgrade "$@" ;;
    service-install) cmd_service_install "$@" ;;
    onboard) cmd_onboard "$@" ;;
    plugin) cmd_plugin "$@" ;;
    run) cmd_run "$@" ;;
    restart) cmd_restart "$@" ;;
    start | stop | status | logs) paperclip_cli service "$cmd" "$@" ;;
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
      "install-canary" "install-prod" "upgrade" "service-install" "onboard" \
      "plugin list" "plugin install" "plugin installed" \
      "start" "run" "stop" "restart" "status" "logs" "uninstall" "help" "quit")" || break
    [[ -n "$pick" ]] || break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      "plugin list") ( dispatch plugin list ) ;;
      "plugin install") ( dispatch plugin install ) ;;
      "plugin installed") ( dispatch plugin installed ) ;;
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
