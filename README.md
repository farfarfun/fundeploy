# nltdeploy

一组面向本机开发与服务管理的 Bash 脚本。

覆盖这些场景：
- 开发环境：`pip` 镜像、`uv`、Python 虚拟环境、Go、Rust、Node.js、pnpm
- 常驻服务：Airflow、Celery、Paperclip、code-server、new-api、sub2api
- 桌面/AI 工具：`nltdeploy ai`（Claude Code、Codex、Cursor）与 OpenPencil（CLI + MCP + Tauri 桌面包）
- 常用工具：gum、GitHub 下载加速、GitHub 网络诊断、按端口杀进程

脚本尽量自包含，可在仓库内直接执行，也可 `curl | bash`。推荐先安装到本地 `~/.local/nltdeploy`，之后只需记住 `nltdeploy`。

## 统一入口

- `nltdeploy`：唯一命令入口；无参数时打开可搜索的分层菜单。
  ```bash
  nltdeploy service status
  nltdeploy service code-server official install
  nltdeploy service code-server official start
  nltdeploy service sub2api official install
  nltdeploy service sub2api official restart
  nltdeploy service sub2api manual install
  nltdeploy dev uv install
  nltdeploy tool github-net doctor
  nltdeploy ai codex update
  nltdeploy upgrade                         # 自动选择本地、GitHub 或 Gitee
  nltdeploy upgrade --source github
  ```
- 领域固定为 `service`、`dev`、`tool`、`ai`，后面依次是模块和动作。
- 不再安装其它 `nlt-*` 兼容命令；旧版本升级时会自动清理这些历史入口。

Homebrew 可通过官方安装器显式安装：

```bash
nltdeploy tool brew install
```

## 安装

### APT

发布仓库启用后，首次添加签名密钥和软件源：

```bash
curl -fsSL https://farfarfun.github.io/nltdeploy/nltdeploy.gpg | sudo tee /usr/share/keyrings/nltdeploy.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/nltdeploy.gpg] https://farfarfun.github.io/nltdeploy stable main" | sudo tee /etc/apt/sources.list.d/nltdeploy.list
sudo apt update
sudo apt install nltdeploy
```

### Homebrew

```bash
brew install farfarfun/tap/nltdeploy
```

### 官方安装脚本

```bash
chmod +x install.sh
./install.sh
./install.sh install
./install.sh update
./install.sh uninstall
```

```bash
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh | bash -s -- install
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/install.sh | bash -s -- install
```

如未自动生效，手动加入 PATH：

```bash
export PATH="$HOME/.local/nltdeploy/bin:$PATH"
```

常用安装变量：
- `NLTDEPLOY_ROOT`：安装根目录
- `NLTDEPLOY_SKIP_GIT_PULL=1`：跳过安装前 `git pull`
- `NLTDEPLOY_SKIP_PROFILE_HINT=1`：不写 shell 配置，适合 CI
- `NLTDEPLOY_UNINSTALL_YES=1`：非交互卸载确认

通过 APT 或 Homebrew 安装时，升级和卸载由对应包管理器负责：

```bash
sudo apt update && sudo apt install --only-upgrade nltdeploy
sudo apt remove nltdeploy
brew upgrade nltdeploy
brew uninstall nltdeploy
```

发行包入口应设置 `NLTDEPLOY_PACKAGE_MANAGER=apt` 或 `brew`；此时 `nltdeploy upgrade` 和 `nltdeploy uninstall` 只显示对应命令，不会修改包管理器维护的目录。

## 领域

### 开发环境

- `nltdeploy dev pip`：pip 镜像配置
- `nltdeploy dev python`：Python/uv 环境
- `nltdeploy dev uv|go|rust|nodejs|pnpm`：语言与包管理器

### 服务

- `nltdeploy service status`：服务总览
- `nltdeploy service airflow|celery|paperclip|code-server|new-api|sub2api|open-pencil`

### 工具

- `nltdeploy tool list`：列出工具
- `nltdeploy tool brew|gum|download|github-net|port-kill|cockpit-tools`

### AI CLI

- `nltdeploy ai claude|codex [official|brew|npm|pnpm] <install|update|uninstall|status>`
- `nltdeploy ai cursor [official] <install|update|uninstall|status>`
- 官方方式始终排在最前且为默认；Cursor 上游目前只提供官方安装器。

## 目录结构

```text
scripts/
  dev/        开发工具统一入口
  tools/      非常驻工具与环境脚本
  services/   常驻服务脚本与 service 聚合入口
  lib/        公共 Bash 库
tests/        冒烟测试
install.sh    安装唯一的 nltdeploy 包装命令
```

## 说明

- 环境：macOS / Linux，Bash 3.2+，通常需要 `curl`
- 模块依赖与环境变量：以各自 `setup.sh` 文件头注释为准
- 冒烟测试：
```bash
bash tests/install_smoke.sh
bash tests/progress_smoke.sh
bash tests/package_smoke.sh
```
- 没执行权限：`chmod +x install.sh` 或对应 `setup.sh`
- PATH 未生效：手动 `export PATH="$HOME/.local/nltdeploy/bin:$PATH"`
- GitHub 下载慢或失败：使用 `nltdeploy tool github-net`、`nltdeploy tool download`

## 详细文档

- [install.sh](install.sh)
- [scripts/dev/README.md](scripts/dev/README.md)
- [scripts/tools/pip-sources/README.md](scripts/tools/pip-sources/README.md)
- [scripts/tools/python-env/README.md](scripts/tools/python-env/README.md)
- [packaging/README.md](packaging/README.md)

## 许可证

[MIT License](LICENSE)
