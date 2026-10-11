# fundeploy

一组面向本机开发与服务管理的 Bash 脚本。

覆盖这些场景：
- 开发环境：`pip` 镜像、`uv`、Python 虚拟环境、Go、Rust、Node.js、pnpm
- 常驻服务：Airflow、Celery、Paperclip、code-server、new-api、sub2api、funmill（任务执行 API）、funflix-web（影视库前后端一体化）、funread-web（阅读源前后端一体化）、funlesson-web（课程备课材料前后端一体化）、funfluid-web（管理端与后台服务前后端一体化）、fungame-web（游戏管理端与客户端前后端一体化）
- 桌面/AI 工具：`fundeploy ai`（Claude Code、Codex、Cursor）与 OpenPencil（CLI + MCP + Tauri 桌面包）
- 常用工具：gum、GitHub 下载加速、GitHub 网络诊断、按端口杀进程

脚本尽量自包含，可在仓库内直接执行，也可 `curl | bash`。推荐先安装到本地 `~/.local/fundeploy`，之后只需记住 `fundeploy`。

## 统一入口

- `fundeploy`：唯一命令入口；无参数时打开可搜索的分层菜单。
  ```bash
  fundeploy service status
  fundeploy service code-server official install
  fundeploy service code-server official start
  fundeploy service sub2api official install
  fundeploy service sub2api official restart
  fundeploy service sub2api manual install
  fundeploy service funflix-web install
  fundeploy service funflix-web start
  fundeploy service funread-web install
  fundeploy service funread-web start
  fundeploy service funlesson-web install
  fundeploy service funlesson-web start
  fundeploy dev uv install
  fundeploy tool github-net doctor
  fundeploy ai codex update
  fundeploy upgrade                         # 沿用安装来源
  fundeploy upgrade --source github
  fundeploy upgrade --source gitee
  ```
- 领域固定为 `service`、`dev`、`tool`、`ai`，后面依次是模块和动作。

Homebrew 可通过官方安装器显式安装：

```bash
fundeploy tool brew install
```

## 安装

### APT

发布仓库启用后，首次添加签名密钥和软件源：

```bash
curl -fsSL https://farfarfun.github.io/fundeploy/fundeploy.gpg | sudo tee /usr/share/keyrings/fundeploy.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/fundeploy.gpg] https://farfarfun.github.io/fundeploy stable main" | sudo tee /etc/apt/sources.list.d/fundeploy.list
sudo apt update
sudo apt install fundeploy
```

### Homebrew

```bash
brew install farfarfun/tap/fundeploy
```

### 官方安装脚本

从本地脚本执行时，`github` 与 `gitee` 二选一：

```bash
chmod +x install.sh
./install.sh install --source github
./install.sh install --source gitee
./install.sh update --source github
./install.sh update --source gitee
./install.sh uninstall
```

远程安装：

```bash
curl -LsSf https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | bash -s -- install --source github
curl -LsSf https://gitee.com/farfarfun/fundeploy/raw/master/install.sh | bash -s -- install --source gitee
```

更新时可沿用已保存的安装来源，也可显式切换：

```bash
fundeploy upgrade
fundeploy upgrade --source github
fundeploy upgrade --source gitee
```

也可直接执行远程更新脚本：

```bash
curl -LsSf https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | bash -s -- update --source github
curl -LsSf https://gitee.com/farfarfun/fundeploy/raw/master/install.sh | bash -s -- update --source gitee
```

安装来源会保存到 `~/.local/fundeploy/etc/fundeploy/source`。显式选择来源后，fundeploy 自身的安装和更新只访问对应站点。

如未自动生效，手动加入 PATH：

```bash
export PATH="$HOME/.local/fundeploy/bin:$PATH"
```

常用安装变量：
- `FUNDEPLOY_ROOT`：安装根目录
- `FUNDEPLOY_SKIP_GIT_PULL=1`：跳过安装前 `git pull`
- `FUNDEPLOY_SKIP_PROFILE_HINT=1`：不写 shell 配置，适合 CI
- `FUNDEPLOY_UNINSTALL_YES=1`：非交互卸载确认

通过 APT 或 Homebrew 安装时，升级和卸载由对应包管理器负责：

```bash
sudo apt update && sudo apt install --only-upgrade fundeploy
sudo apt remove fundeploy
brew upgrade fundeploy
brew uninstall fundeploy
```

发行包入口应设置 `FUNDEPLOY_PACKAGE_MANAGER=apt` 或 `brew`；此时 `fundeploy upgrade` 和 `fundeploy uninstall` 只显示对应命令，不会修改包管理器维护的目录。

## 领域

### 开发环境

- `fundeploy dev pip`：pip 镜像配置
- `fundeploy dev python`：Python/uv 环境
- `fundeploy dev uv|go|rust|nodejs|pnpm`：语言与包管理器

### 服务

- `fundeploy service status`：服务总览
- `fundeploy service airflow|celery|paperclip|code-server|new-api|sub2api|funmill|open-pencil|funflix-web|funread-web|funlesson-web|funfluid-web|fungame-web`
- `fundeploy service paperclip install|install-canary|check|upgrade|rollback|doctor|health|channels|onboard|service-install|plugin|start|run|stop|restart|status|logs|uninstall`：
  全部直驱 Paperclip 官方 CLI（`npx paperclipai@latest install --yes` 的 managed install，即
  `INSTALLING.md` 记载的正式路径），不引入官方 `install.sh` —— `paperclip.ing` 上那份是过期快照
  （给 CLI 传了它不接受的 `--no-prompt`、`MIN_NODE_MAJOR=20`、用 bash 4 语法在 macOS 自带 bash 3.2 上崩），
  master 上那份正确的没有发布 `.sha256`，且它的 Node 引导见到 `~/.nvm` 就直接硬退出
- `check` 只查版本不改动任何东西；`upgrade [canary|prod]` 是「备份 → 停服 → 官方 `update` →
  `doctor --repair` → 重启 → 健康检查」；canary 机器上 `upgrade prod` 是降级，需交互确认或
  `PAPERCLIP_UPGRADE_YES=1`；`rollback` 回退到 install store 里保留的上一份 payload
- `start|stop|restart|status|logs` 优先转发到 Paperclip 官方 `service` 命令（Linux systemd user /
  macOS LaunchAgent）；检测不到服务管理器的机器（容器、WSL1、无登录会话）上官方命令只会打印一句
  「不支持」就以 0 退出、什么都不启动，此时自动改由 fundeploy 用 nohup + PID 文件托管后台进程并等
  `/api/health` 通过，代价是不开机自启（`PAPERCLIP_FALLBACK=off` 可关掉这条兜底通道；想要开机自启
  就在有登录会话的 shell 里执行 `service-install --enable-linger`）
- 受限网络：官方 managed install 把 registry 钉死在 `registry.npmjs.org`（刻意的防投毒设计，有上游
  测试守着），换镜像无效，只能走 `PAPERCLIP_HTTPS_PROXY`；依赖树实测约 1.4G
- `fundeploy service funflix-web install|upgrade|rollback <版本>|start|run|stop|restart|status|migrate|user-create|uninstall`：
  funflix-api（后端，PyPI，依赖核心库 funflix）+ funflix-web（前端，私有 npm）一起装/起/停，只提供合并命令。
  新机器装完先 `migrate`（建表）再 `user-create <用户名>`（建登录账号）——1.0.30 起是整站门禁，没账号登不进去
- `fundeploy service funread-web install|upgrade|rollback <版本>|start|run|stop|restart|status|uninstall`：
  funread-api（后端，PyPI，依赖核心库 funread）+ @farfarfun/funread-web（前端，私有 npm）一起装/起/停，只提供合并命令。
  浏览器只访问前端端口（阅读端 `/web`，管理端 `/admin`），前端把 `/api` 与 `/healthz` 反代到后端，直连后端会撞 CORS。
  新机器装完先 `funread-api accounts create-admin <用户名>` 建管理员——注册由邀请码控制，没码注册不进去。
  两个 CLI 的 `server start` 都要求自己的 `config.toml` 已存在（1.1.111–1.1.113 上缺了只报一句
  `config file not found`），`install` / `start` / `run` 会在默认路径下按需补一份；
  显式设了 `FUNREAD_WEB_{BACKEND,FRONTEND}_CONFIG` 的路径不代建。
  默认路径问 CLI 自己（`server status` 会打 `config <路径>`），不按版本猜：funread 1.1.114 把配置目录
  从 `${XDG_CONFIG_HOME:-~/.config}/farfarfun/funread-{api,web}/` 换到了 `~/.farfarfun/funread/{api,web}/`，
  而 `rollback 1.1.113` 能把机器退回旧布局。
  `upgrade` 与 `rollback` 都会先停服再换包、装完按原状态起回来——在跑着的进程底下换全局 npm 包，
  node 还开着那棵树里的文件；存活判断走端口探测，因为 PID 文件的位置本身会随版本变
- `fundeploy service funlesson-web install|start|run|stop|restart|status|uninstall`：funlesson-api（后端，私有
  PyPI）+ funlesson-web（前端，私有 npm）一起装/起/停，只提供合并命令
- `fundeploy service funfluid-web install|start|run|stop|restart|status|uninstall`：funfluid-api（后端）
  + funfluid-web（前端）一起装/起/停，只提供合并命令
- `fundeploy service fungame-web install|start|run|stop|restart|status|uninstall`：fungame 后端
  + 管理端 + 客户端三个进程一起装/起/停

所有服务默认只监听回环地址（`127.0.0.1`）。需要对外暴露时显式设置对应的
`*_HOST` / `*_ADDRESS` 环境变量（如 `SUB2API_HOST=0.0.0.0`、`FLOWER_ADDRESS=0.0.0.0`），
脚本会在启动时提示确认防火墙与访问控制。

### 工具

- `fundeploy tool list`：列出工具
- `fundeploy tool brew|gum|download|github-net|skills-sync|port-kill|cockpit-tools`

### AI CLI

- `fundeploy ai claude|codex [official|brew|npm|pnpm] <install|update|uninstall|status>`
- `fundeploy ai cursor [official] <install|update|uninstall|status>`
- 官方方式始终排在最前且为默认；Cursor 上游目前只提供官方安装器。

## 目录结构

```text
scripts/
  dev/        开发工具统一入口
  tools/      非常驻工具与环境脚本
  services/   常驻服务脚本与 service 聚合入口
  lib/        公共 Bash 库
tests/        冒烟测试
install.sh    安装唯一的 fundeploy 包装命令
VERSION       当前发布版本
```

## 说明

- 环境：macOS / Linux，Bash 3.2+，通常需要 `curl`
- 模块依赖与环境变量：以各自 `setup.sh` 文件头注释为准
- 冒烟测试：
```bash
bash tests/run-all.sh
```
- 没执行权限：`chmod +x install.sh` 或对应 `setup.sh`
- PATH 未生效：手动 `export PATH="$HOME/.local/fundeploy/bin:$PATH"`
- GitHub 下载慢或失败：使用 `fundeploy tool github-net`、`fundeploy tool download`

## 详细文档

- [install.sh](install.sh)
- [scripts/dev/README.md](scripts/dev/README.md)
- [scripts/tools/pip-sources/README.md](scripts/tools/pip-sources/README.md)
- [scripts/tools/python-env/README.md](scripts/tools/python-env/README.md)
- [packaging/README.md](packaging/README.md)

---

## 关于 farfarfun

[farfarfun](https://github.com/farfarfun) 是一个专注于实用工具库的开源组织，
涵盖云存储、数据处理、AI、多媒体与开发工具链等方向。

- 🏠 组织主页：<https://github.com/farfarfun>
- 📧 联系：farfarfun@qq.com

本项目基于 [MIT](LICENSE) 协议开源。
