# nltdeploy

一组面向本机开发与服务管理的 Bash 脚本。

覆盖这些场景：
- 开发环境：`pip` 镜像、`uv`、Python 虚拟环境、Go、Rust、Node.js、pnpm
- 常驻服务：Airflow、Celery、Paperclip、code-server、new-api、sub2api
- 桌面/AI 工具：`nlt-ai-cli`（Claude Code、Codex、Cursor 等 CLI 工具）与 OpenPencil（CLI + MCP + Tauri 桌面包）
- 常用工具：gum、GitHub 下载加速、GitHub 网络诊断、按端口杀进程

脚本尽量自包含，可在仓库内直接执行，也可 `curl | bash`。推荐先安装到本地 `~/.local/nltdeploy`，之后统一用 `nlt-*` 命令。

## 统一入口

- `nltdeploy`：整个项目的顶层入口。
  ```bash
  nltdeploy upgrade                     # 升级本安装（自动选源：本地→github→gitee）
  nltdeploy upgrade --source github     # 指定源：github / gitee / local
  nltdeploy uninstall                   # 卸载 nltdeploy
  nltdeploy tools gum install           # 透传给 nlt-tools
  ```
- `nlt-tools`：工具类统一入口，每个工具支持 `install`（幂等，检测→未装则装）/ `upgrade` / `uninstall`。
  ```bash
  nlt-tools list
  nlt-tools gum install
  nlt-tools gum upgrade
  nlt-tools python-env install
  nlt-tools go uninstall
  ```
  服务脚本内部依赖某工具时，不自行检测，直接 `nlt-tools <工具> install`。
  工具名：`gum download pip-sources python-env github-net port-kill cockpit-tools go rust nodejs pnpm uv`。

## 安装

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

## 主要模块

### 开发环境

- `nlt-dev`：开发环境统一入口
- `nlt-pip-sources`：pip 镜像配置
- `nlt-python-env`：Python/uv 环境

### 服务

- `nlt-airflow`
- `nlt-celery`
- `nlt-paperclip`
- `nlt-code-server`
- `nlt-new-api`
- `nlt-sub2api`
- `nlt-open-pencil`（OpenPencil CLI + MCP，可选桌面 Tauri 安装包）
- `nlt-services`

### 工具

- `nlt-utils`
- `nlt-github-net`
- `nlt-download`
- `nlt-cockpit-tools`
- `nlt-port-kill`

### AI CLI

- `nlt-ai-cli`：统一安装 / 更新 / 卸载 Claude Code、Codex、Cursor 等 CLI 端 AI 工具；只走官方 package / installer，不支持源码安装。

## 目录结构

```text
scripts/
  dev/        开发工具统一入口
  tools/      非常驻工具与环境脚本
  services/   常驻服务脚本与 nlt-services 聚合入口
  lib/        公共 Bash 库
tests/        冒烟测试
install.sh    安装 nlt-* 包装命令
```

## 说明

- 环境：macOS / Linux，Bash 3.2+，通常需要 `curl`
- 模块依赖与环境变量：以各自 `setup.sh` 文件头注释为准
- 冒烟测试：
```bash
bash tests/install_smoke.sh
bash tests/progress_smoke.sh
```
- 没执行权限：`chmod +x install.sh` 或对应 `setup.sh`
- PATH 未生效：手动 `export PATH="$HOME/.local/nltdeploy/bin:$PATH"`
- GitHub 下载慢或失败：优先看 `nlt-github-net`、`nlt-download`

## 详细文档

- [install.sh](install.sh)
- [scripts/dev/README.md](scripts/dev/README.md)
- [scripts/tools/pip-sources/README.md](scripts/tools/pip-sources/README.md)
- [scripts/tools/python-env/README.md](scripts/tools/python-env/README.md)
- [pyproject.toml](pyproject.toml)

## 许可证

[MIT License](LICENSE)
