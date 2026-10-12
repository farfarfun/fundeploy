# Changelog

版本号以根目录 `VERSION` 为准；发布由推 `v<VERSION>` tag 触发（见
`.github/workflows/release.yml`），因此「有 tag ＝ 已发布」。

## [0.1.18]

### 新增

- `paperclip`：补上上游早已有、而本脚本一直没暴露的一等公民命令：
  `check [canary|prod]`（只查版本不改动任何东西）、`rollback`（回退到 install store
  里保留的上一份 payload）、`backup`（官方 `db:backup`）、`doctor [--repair]`、
  `health`、`channels`，以及
  `service-install`（透传 `--enable-linger` / `--no-start-on-login` /
  `--no-start-now`——此前 fundeploy 管理的 Paperclip 根本不会开机自启：`start` 内部的
  `ensureCurrent()` 会自愈 systemd 单元文件，但 `systemctl --user enable` 和
  `loginctl enable-linger` 只有官方 `service install` 会做）。
- `paperclip`：新增 `PAPERCLIP_UPGRADE_YES=1`，用于在非交互环境确认降级。

- `funread-web`：新增 `upgrade`（装完额外打印前后端的版本对比）与
  `rollback <版本>`（前后端一起钉到同一版本，回退前在跑就装完自动起回来）。
  `rollback` 会先用 `npm view` 确认私有源上真有这个版本再动安装——`install` 的
  顺序是先后端后前端，前端拉不到而后端已经降级的话，会停在前后端版本不一致的
  中间态上。npm 上 `@farfarfun/funread-web` 的版本号是跳着的（`1.1.106`–`1.1.108`
  三次发版都因到 Codeup registry 的链路丢包而从未发布成功），所以这个前置检查
  不是多余的。

### 修复

- `paperclip`：`upgrade` 把官方 `update --check --json` 的退出码 **10** 当成了失败。
  这个退出码是「有可用更新」的信号（payload 里 `if (options.check) process.exitCode = 10`，
  性质同 `diff`），不是错误——于是唯一真正需要升级的那种情况反而被判成「拿不到版本
  信息」。现在只看 stdout 能不能解析成 JSON，不看返回值。顺带给 `paperclip_cli` 加了
  `PAPERCLIP_CLI_NO_DIAGNOSE`，免得这种「非零但成功」的调用白跑一次损坏探测。
- `paperclip`：canary 机器上 `upgrade prod` 会卡死。`--latest` 指向的 stable 往往比
  当前 canary 版本旧（实测本机 `2026.1008.0-canary.4` → `2026.1005.0`），官方 `update`
  把这种情况当显式降级、要求 `-y` 确认，而此前脚本既不检测也不传，非交互环境里就停在
  上游的交互提示上。现在先问 `update --check --json`：`downgrade:true` 时经
  `fundeploy_confirm_destructive` 确认后才补 `-y`，`updateAvailable:false` 时直接跳过
  （不再白停一次服务）。
- `paperclip`：无服务管理器时 fundeploy 托管的后台进程缺 `</dev/null`，会继承调用者的
  终端 stdin，在交互式 shell 里一读 stdin 就被 `SIGTTIN` 停住，表现为「起来了但没响应」。
- `paperclip`：消毒用的临时 npmrc 会泄漏到 `/tmp`。它此前在脚本顶层无条件创建并挂
  `trap … EXIT`，而 `run` / `logs` 这些路径以 `exec` 收尾，`exec` 换掉进程映像后 EXIT
  trap 不会触发。现在改成只在 `install` / `upgrade` / `plugin install` 期间按官方
  `install.sh` 的做法写一份仅含两行 registry 的最小 npmrc（同时也是 npm allow-scripts
  缺陷 npm/cli#9783 的官方同款解法，替掉了原来那段 grep 式 `~/.npmrc` 消毒），并在
  `exec` 之前显式清理。
- `fundeploy service status`：Paperclip 的端口兜底值 `3100` 是错的，改为 `8804`——
  config.json 的 `server.port` 和 `paperclipai health` 默认探的都是 8804。配置还没生成
  时这里会去探一个根本没人监听的端口，把「未安装」显示成「已停止」。

- `funread-web`：`install` / `start` / `run` 会在默认配置目录下按需补一份
  `config.toml`，不补的话新机器上 `start` 必然失败。funread 1.1.111 的两个 CLI
  共享同一处缺陷：`server start` 把它自己解析出来的**默认** config 路径原样以
  `--config` 传给 fork 出的子进程，而上游对「显式指定的 config 不存在」是硬错误
  （默认路径不存在本来是容许的）。于是 `fundeploy service funread-web install`
  成功之后紧跟的 `start` 退出码 1，日志里只有一行
  `error: config file not found: ~/.config/farfarfun/funread-api/config.toml`，
  看着像包装坏了；而同一份安装 `funread-api server run` 一切正常。上游既没有
  `init` 也没有 `config` 子命令能生成这个文件，所以由部署侧补上——性质同
  funflix-web 当初补的「首次部署两步」。生成的内容只有 `[server]` 下的 host /
  port（真正生效的仍是 `start` 显式传的 `--host`/`--port`，这份文件主要是让上游
  的存在性检查过关），**不写库地址**——库地址属于两端共用的配置，写在上一层的
  `~/.farfarfun/funread/config.toml`（见下一条）。显式设置了
  `FUNREAD_WEB_{BACKEND,FRONTEND}_CONFIG` 时不代建：用户点名了那个文件，替他凭空
  造一份只会把「路径写错了」盖掉。

- `funread-web`：install/start/run 另外生成一份共享根配置
  `~/.farfarfun/funread/config.toml`（`_ensure_shared_config`）。funread 1.1.116 起
  库地址、管理端口令这些只认配置文件，`FUNREAD_DATABASE_URL` 等环境变量整个不读了，
  所以必须有个地方摆给操作者填。生成的内容**整份是注释**：填一个 `[database] url`
  进去就是决定这台机器连哪个库，脚本没有资格替操作者定；全注释的文件行为上等于
  没有这个文件（仍按 funsecret > 本地 SQLite 解析）。已存在则一个字都不动——那份
  文件里放的正是生产库地址和口令，升级时覆盖它等于静默把线上指到别的库上。
  权限 `0600`，两份服务配置也一并从 `0644` 收到 `0600`。

- `funread-web`：上面那份 `config.toml` 写到哪个目录，改成问 CLI（`server status`
  的输出里带 `config <路径>`，服务没在跑也会打），不再按 funread 的布局自己拼。
  funread 1.1.114 把配置目录从 `${XDG_CONFIG_HOME:-~/.config}/farfarfun/funread-{api,web}/`
  换到了 `~/.farfarfun/funread/{api,web}/`，而同一个 fundeploy 要同时伺候两种版本的
  机器——光是 `rollback 1.1.113` 就能把一台机器退回旧布局。两边任选一个写死，都会让
  这份配置建到 CLI 根本不看的位置，症状和它要修的那个缺陷一模一样（`start` 报
  `config file not found`），而且更难查：日志里明明有一行「已生成配置」。CLI 问不出来
  （没装、或上游改了输出格式）才退到新布局的硬编码值。

- `funread-web`：`upgrade` 改为「先停服 → 装新版 → 打印版本对比 → 原先在跑就起回来」，
  与 `rollback` 一致。此前它在跑着的进程底下直接换包：前端是全局 npm 包，node 还开着
  那棵树里的文件，装一半的产物会被新请求读到；后端换 uv tool / pip 环境同理。
  1.1.113 → 1.1.114 这一跳还多一个一次性的坑——旧进程的 PID 文件写在**旧**配置目录里，
  升级后的 CLI 只看新目录，于是 `stop` 报「not running」而端口还被占着，接着 `start`
  又撞 address in use。存活判断走端口探测而不是 PID 文件，正是因为 PID 文件的位置本身
  会随版本变，端口不会。

- `funread-web`：「先停服」带来的新失败模式——停完之后换包挂掉，`set -e` 让脚本就地
  退出，用户只看到包管理器那几行报错，没有一句说机器上的服务现在是停的。实测撞到过：
  私有 npm 源一个 `ETIMEDOUT`，结果后端已经换成新版、前端还是旧版、两个端口都没人听，
  而输出里一个字都没提服务状态。`upgrade` / `rollback` 现在在停服后挂一个 EXIT 钩子，
  换包没走完就退出时明确提示「现在是停着的」并给出恢复办法；换包成功后立刻撤掉，
  免得后面 `start` 自己的报错上再附一句无关的。

- `fundeploy service status`：`FUNREAD_WEB_SERVICE_HOME` 在「安装路径」那行仍被
  展开，但它的赋值已随 funread-web 改走正式包而删掉了，`set -u` 下该行会以
  `unbound variable` 中断整条命令：状态表已经打出来了，但「安装路径」及其后的
  「详情」「工具」三行全丢，退出码变成 1——拿退出码判断的调用方会把一次正常的
  status 当成失败。funread-web 不再有安装目录（后端在 uv tool / pip 的环境里，
  前端是全局 npm 包），已从该行移除，与同样装正式包的 funflix-web /
  funlesson-web 一致。

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

- **破坏性**：`paperclip` 整体重构为「直驱官方 CLI」，删掉约 430 行自造机制
  （1240 行 → 1074 行，其中代码 742 行，与兄弟脚本的 589 / 614 持平）。
  删掉的两大块：
  （1）约 295 行 npm 镜像预装机制。它要解决「依赖树 1.4G 下得慢」，但上游把
  `registry.npmjs.org` 在 `install.sh` 和 `cli/src/commands/install.ts` 两层硬钉死
  （`--registry=`、`--@paperclipai:registry=`、四种 `NPM_CONFIG_*` 拼写、一次性
  `npm_config_userconfig`），这是有上游测试守着的防 registry 投毒设计，任何 flag 和
  环境变量都换不掉镜像——这套机制在解决一个它解决不了的问题，而且自己带来了 bug。
  （2）约 130 行 macOS arm64 embedded-postgres 补丁（pnpm `allowBuilds`、
  `hydrate-symlinks`、删 launchd 里的 `DATABASE_URL`），改为调用上游现成的
  `doctor --repair --yes`（它会检查配置、install store、manifest、`current` 链接、
  shim、PATH、目录权限等并尝试修复）；刻意放在 `start` 之前，因为官方
  `paperclipai run` 自己也是 `onboard` + `doctor --repair` 之后才启动。
  手搓的版本比较与「already up to date」判断也交回 `update --check --json`。
  **迁移**：`PAPERCLIP_NPM_MIRROR` 与 `PAPERCLIP_NPM_PREFETCH` 两个变量已移除
  （设了也从来没能改变实际下载源）。受限网络改用 `PAPERCLIP_HTTPS_PROXY` +
  `PAPERCLIP_NO_PROXY`——那是唯一能透进官方 managed install 的通道；
  `PAPERCLIP_NPM_FETCH_*`（取回重试与超时）和 `PAPERCLIP_INSTALL_VIA_NPX` 保留。
- **破坏性**：`paperclip` 的 `upgrade` / `rollback` 不再自动备份数据库，备份提成独立的
  `backup` 命令（转调官方 `db:backup`）。`update` 换的是 payload，不会动
  `instances/<id>/db` 里的数据，payload 要退回去有 `rollback`；而一次备份在实测机器上
  是 370M、耗时数十秒，每次升级都付这个代价并不划算。给官方 `update` 一并传了
  `--no-backup`，把它自己那次 pre-update 备份也关掉。顺带去掉了「服务停着时升级会在第
  一步中止」这个失败模式——embedded PostgreSQL 是服务的子进程、随服务一起停，停着时
  `db:backup` 必然连不上，`set -e` 下整条升级就断在那里；现在没有这一步了。
  **迁移**：原先依赖升级自动留一份备份的，改为先 `backup` 再 `upgrade`。embedded
  PostgreSQL 模式下 `backup` 要求服务在跑，停着时会在调用上游之前就报错并提示先
  `start`，而不是丢一个连接错误出来；用外部数据库的实例与服务生命周期无关，停着也能备。
- `paperclip`：仍不采用官方文档推荐的 `curl -O https://paperclip.ing/install.sh`
  + sha256 引导路径，因为那条路径当前是坏的：`paperclip.ing` 上那份（文档指向、
  也是唯一发布了 `.sha256` 的）落后于 master 的 #11792，给 CLI 传了 `install` 并不
  接受的 `--no-prompt`（交接直接失败）、`MIN_NODE_MAJOR=20`（上游已要求 24.11+）、
  用 `${value,,}` 这种 bash 4 语法在 macOS 自带 bash 3.2 上崩、第三方脚本无 SHA 固定
  （上游 issue #10739 在跟踪）；master 上那份正确的 `scripts/install.sh` 没有发布
  checksum（`.sha256` 返回 404），无从校验；它的 Node 引导还会在见到 `~/.nvm` 时
  直接 `fail`。而只要 PATH 上已有 node 24+，install.sh 的 Node 引导块会被整个跳过，
  它就只剩 `npx --yes --registry=… paperclipai@latest install --yes` 这一行——也正是
  本脚本执行的那条。终态与 install.sh 一致，但不引入一个过期、且在装了 nvm 的机器上
  根本跑不起来的脚本依赖。
- `paperclip`：`nvm use 24` 不再在每次 CLI 调用前执行。managed install 的 shim
  已经用绝对路径把自己那份 node 钉死（`exec '<nvm>/…/v24.x/bin/node' '<payload>/dist/index.js'`），
  经 shim 的调用自带正确的 node；版本门槛因此收窄到真正需要它的 npx 引导路径
  （那时还没有 shim）与 `run` / `start`，`status` / `logs` / `stop` 这些热路径不再
  每次 `source nvm.sh` 并改写 PATH。
- `paperclip`：交互菜单的选项补成「命令 + 中文描述」并用 `pick="${pick%% *}"` 还原，
  与仓库其余服务一致（此前是裸命令名，是全仓唯一的偏离）。
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
- **破坏性**：`funread-web` 从「源码构建」改为「装正式包 + 转发上游 CLI」，与
  funflix-web / funlesson-web 对齐。后端由 `funread[api]` 装进脚本自建的 venv
  改为装 PyPI 上的 `funread-api`（uv tool / pip 自适应，复用机器上已有的那一份）；
  前端由 `git clone` + `pnpm run build` 改为装私有 npm 上的
  `@farfarfun/funread-web`。PID 与日志不再由本脚本 `nohup` 托管，改由两个 CLI
  各自写在实际解析到的 config 同目录下，`start`/`run`/`stop`/`status` 只是转发
  `funread-api server …` 与 `funread-web server …`。
  **迁移**：`FUNREAD_WEB_SERVICE_HOME` / `FUNREAD_WEB_GIT_URL` /
  `FUNREAD_WEB_GIT_REF` / `FUNREAD_WEB_PYTHON_BIN` 四个变量已移除，改用
  `FUNREAD_WEB_{BACKEND,FRONTEND}_{PACKAGE,VERSION,CONFIG}`；旧的
  `~/opt/funread-web` 不会被自动清理，确认服务已按新路径起得来之后手动删掉即可。
  前端在私有源上，首次安装前机器上需要一次性的 scope 映射
  （`npm config set @farfarfun:registry …`，pnpm 不接受命令行覆盖）。
  `install`/`upgrade`/`rollback` 故意不转调上游 CLI 的同名命令：`funread-api upgrade`
  写死 `uv tool install`、`funread-web upgrade` 写死 `npm install -g`，在用 pip /
  pnpm 装过的机器上会各装出第二份副本，PATH 里仍是旧版——升级看着成功其实没生效。
  `install` 后会核对「刚装的那份是不是 PATH 上会跑到的那份」，不一致就告警。
- `funflix-web`：前端 npm 包名跟随上游改为 `@farfarfun/funflix-web`（bin 名仍是
  `funflix-web`，与 `funfluid-web` 的写法对齐）。`install` / `upgrade` 会先摘掉
  改 scope 前装的裸名全局包——它占着 `bin/funflix-web`，不清掉新包会在建 bin
  软链时直接 `EEXIST` 失败。显式把 `FUNFLIX_WEB_FRONTEND_PACKAGE` 指回裸名时
  不做这个清理。

### 废弃

- `paperclip`：`PAPERCLIP_NPM_MIRROR` / `PAPERCLIP_NPM_PREFETCH` 已移除（见「变更」
  里的迁移说明）。`update` 作为 `upgrade` 的别名继续保留。

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
