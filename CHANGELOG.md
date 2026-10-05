# Changelog

版本号以根目录 `VERSION` 为准；发布由推 `v<VERSION>` tag 触发（见
`.github/workflows/release.yml`），因此「有 tag ＝ 已发布」。

## [0.1.18]

### 新增

- 无

### 修复

- `celery`：Flower 的 basic auth 凭据不再以 `--basic-auth=user:pass` 形式进入
  命令行。守护进程存活期间该参数一直在 `/proc/<pid>/cmdline` 里，而这个文件对
  本机所有用户可读，`ps aux | grep flower` 即可读到明文（SPEC §9.1）。改为
  `export FLOWER_BASIC_AUTH`，由 Flower 自己的 `FLOWER_*` 环境变量机制读取。
- `celery`：`run-flower`（前台）此前既不传认证也不告警，用户设了
  `FLOWER_BASIC_AUTH` 仍会跑出一个完全无认证的 Flower。认证与「对外监听且无
  认证」的告警统一下沉到 `prepare_flower_env()`，`start-flower` 与 `run-flower`
  行为一致。
- `sub2api`：`sub2api.env` 里用户自己写的 `SERVER_HOST` / `SERVER_PORT` 不再被
  脚本的代码默认值覆盖。按 SPEC §9.3 的「环境变量 > 配置文件 > 代码内默认值」，
  只有显式设置了 `SUB2API_HOST` / `SUB2API_PORT` 时才覆盖配置文件的值。
- `fundeploy service status`：Celery 行里残留的 `FLOWER_ADDRESS` 默认值
  `0.0.0.0` 与 `services/celery/setup.sh` 的实际默认值 `127.0.0.1` 不一致，
  会展示出一个服务其实不会绑定的监听地址，已对齐。

### 变更

- **破坏性**：`sub2api` 默认监听地址由 `0.0.0.0` 改为 `127.0.0.1`。sub2api 是
  订阅 API 网关，`data/` 下存着上游渠道 key 与用户 token，默认绑全网卡不符合
  SPEC §9.3 的安全默认值要求；同仓库其余服务默认都是 `127.0.0.1`，
  `fundeploy service status` 的探活逻辑也一直按 `127.0.0.1` 写。
  **迁移**：需要对外暴露时显式设置 `SUB2API_HOST=0.0.0.0`（或在
  `~/opt/sub2api/data/sub2api.env` 里写 `SERVER_HOST=0.0.0.0`），`start` / `run`
  会提示确认防火墙与访问控制。
- `celery`：虚拟环境与依赖改为优先走 `uv`（`uv venv` + `uv pip install`，与同
  仓库 airflow 服务一致），缺 `uv` 时退回 `python3 -m venv` + `pip`；依赖不再是
  裸包名，改为带版本下限的 `celery>=5.3` / `redis>=5.0` / `flower>=2.0`，可用
  `CELERY_PKG_SPEC` / `CELERY_REDIS_PKG_SPEC` / `CELERY_FLOWER_PKG_SPEC` 覆盖
  （SPEC §5）。
- README：底部组织介绍区块补上 SPEC §12.2 要求的 `---` 分隔线，并去掉与区块末行
  重复的独立「许可证」小节；服务清单补上 `funmill` / `funfluid-web` /
  `fungame-web`，并写明「默认只监听回环」的约定。
- CHANGELOG：修正 `[0.1.15]` 的错误归属（见下）。
- `funflix-web`：前端 npm 包名跟随上游改为 `@farfarfun/funflix-web`（bin 名仍是
  `funflix-web`，与 `funfluid-web` 的写法对齐）。`install` / `upgrade` 会先摘掉
  改 scope 前装的裸名全局包——它占着 `bin/funflix-web`，不清掉新包会在建 bin
  软链时直接 `EEXIST` 失败。显式把 `FUNFLIX_WEB_FRONTEND_PACKAGE` 指回裸名时
  不做这个清理。

### 废弃

- 无

## [0.1.17] - 2026-09-27

### 新增

- 无

### 修复

- `_pip_install_pkg`：修正 `uv pip install` / `python3 -m pip install` 的参数
  拼接，此前的写法在部分路径下会把包名与选项拼错。
- `fungame-web` / `funlesson-web` / `funflix-web`：`uv pip install` 补回 `-U`，
  否则已安装过的包不会被升级到目标版本。

### 变更

- 无

### 废弃

- 无

## [0.1.16] - 未发布

> 没有 `v0.1.16` tag，也没有对应 GitHub Release；`VERSION` 曾短暂停在
> `0.1.16`，这些改动实际随 `0.1.17` 一起发布。

### 新增

- 新增 `funfluid-web` 服务模块（install / start / stop / restart / status）。
- 新增 `fungame-web` 服务模块（后端 + 管理端 + 客户端三进程编排）。
- 新增 `funlesson-web` 服务模块（前后端一体化管理）。
- 新增 `sync-gitee` workflow，每次 push 镜像到 Gitee。
- README 补充「关于 farfarfun」组织介绍区块，末尾附 MIT 协议说明句。

### 修复

- `paperclip`：改为从 npm userconfig 里清掉 `allow-scripts`，只清环境变量不够。
- 前端类服务：修正 pnpm 的 `@latest` dist-tag 解析问题。
- `funfluid-web`：显式导出 `FUNFLUID_CONFIG_DIR` / `FUNFLUID_DATA_DIR`。
- `funflix-web`：后端改用 `funflix-api`（跟随 funflix 仓库拆分）。
- `sync-gitee`：只推 heads/tags，GitHub 的 PR ref 会被 Gitee 当隐藏 ref 拒绝。
- `install_smoke`：收紧 PATH 时保持 node 可达，否则服务 status 用例假失败。
- `.gitignore` 补齐 `__pycache__/`、`*.pyc`、`*.db`、`*.rar`、`.venv/`、`.run/`、
  `node_modules/` 等规范要求的规则。

### 变更

- 各服务模块的 `update` 子命令统一改名为 `upgrade`。
- 前端包安装优先用 pnpm，没有 pnpm 时退回 npm。
- `paperclip` 改为复用上游自带的服务管理命令。

### 废弃

- 服务模块的 `update` 子命令（由 `upgrade` 取代）。

## [0.1.15] - 2026-08-11

### 变更

- 无（见下方说明）

> 更正：原先记在本版本下的「README 组织介绍区块」与「`.gitignore` 补齐」两条
> 实际是 2026-09-03 的提交，晚于 `v0.1.15`（2026-08-11）—— `v0.1.15` 这个 tag 的
> 树里既没有 `CHANGELOG.md`，README 也还是旧的「许可证 / [MIT License](LICENSE)」
> 结尾。两条已移到实际落地的 `[0.1.16]` 下。

---

> 关于 SPEC §6「服务脚本」：`scripts/setup.sh` 统一入口、`start`/`run` 强制带
> `dev`/`prod`、运行时文件迁到仓库内 `.run/` 这三项，在本仓库不适用，已在
> todo-list#383 与 #665 中说明理由并留待仓库所有者确认：
>
> - fundeploy 自身不含任何长期运行服务。`scripts/services/*/setup.sh` 管的是
>   **安装在用户本机 `~/opt/<service>/` 下的第三方 / 其他仓库的服务**，
>   fundeploy 只是部署器。
> - fundeploy 以 apt / Homebrew / `~/.local/fundeploy` 形式分发，运行时根本不存在
>   「仓库工作区」，把被管服务的 PID/日志写到仓库内 `.run/` 既无处可写也会丢失。
> - 被管服务没有天然的 dev/prod 划分：主机上只有一份正式安装（PyPI / npm /
>   GitHub Release 的发布产物），不存在「从源码起」这条路径。
> - 统一入口已经有了，是 `fundeploy` 命令本身（`scripts/fundeploy.sh`，
>   `域 → 模块 → 动作`），而不是 `scripts/setup.sh`。
