# fundeploy

一组面向本机开发与服务管理的 Bash 脚本。

覆盖这些场景：
- 开发环境：`pip` 镜像、`uv`、Python 虚拟环境、Go、Rust、Node.js、pnpm
- 常驻服务：Airflow、Celery、Paperclip、code-server、new-api、sub2api、funflix-web（影视库前后端一体化）
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
- `fundeploy service airflow|celery|paperclip|code-server|new-api|sub2api|open-pencil|funflix-web`
- `fundeploy service funflix-web install|start|stop|restart|status|uninstall`：funflix（后端，PyPI）+
  funflix-web（前端，私有 npm）一起装/起/停，只提供合并命令

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

## 许可证

本项目基于 [MIT](LICENSE) 协议开源。

## 关于 farfarfun

[farfarfun](https://github.com/farfarfun) 是一个专注于实用工具库的开源组织，
涵盖云存储、数据处理、AI、多媒体与开发工具链等方向。

- 🏠 组织主页：<https://github.com/farfarfun>
- 📧 联系：farfarfun@qq.com
