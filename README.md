# nltdeploy

一组面向本机开发与服务管理的 Bash 脚本。

覆盖这些场景：
- 开发环境：`pip` 镜像、`uv`、Python 虚拟环境、Go、Rust、Node.js、pnpm
- 常驻服务：Airflow、Celery、Paperclip、code-server、new-api、sub2api
- 常用工具：gum、GitHub 下载加速、GitHub 网络诊断、按端口杀进程

脚本尽量自包含，可在仓库内直接执行，也可 `curl | bash`。推荐先安装到本地 `~/.local/nltdeploy`，之后统一用 `nlt-*` 命令。

## 安装

本地仓库安装：

```bash
chmod +x install.sh
./install.sh
./install.sh install
./install.sh update
./install.sh uninstall
```

远程安装：

```bash
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh | bash -s -- install
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/install.sh | bash -s -- install
```

安装后默认会生成：
- `~/.local/nltdeploy/libexec/nltdeploy`
- `~/.local/nltdeploy/bin`

如未自动生效，手动加入 PATH：

```bash
export PATH="$HOME/.local/nltdeploy/bin:$PATH"
```

常用安装变量：
- `NLTDEPLOY_ROOT`：安装根目录
- `NLTDEPLOY_SKIP_GIT_PULL=1`：跳过安装前 `git pull`
- `NLTDEPLOY_SKIP_PROFILE_HINT=1`：不写 shell 配置，适合 CI
- `NLTDEPLOY_UNINSTALL_YES=1`：非交互卸载确认

## 常用命令

```bash
nlt-dev                 # 开发环境统一入口
nlt-dev pip             # pip 镜像
nlt-dev uv              # 安装/升级 uv
nlt-dev python          # 创建 Python/uv 环境

nlt-airflow             # Airflow 菜单
nlt-celery              # Celery 菜单
nlt-paperclip           # Paperclip 菜单
nlt-code-server         # code-server 菜单
nlt-new-api             # new-api 菜单
nlt-sub2api             # sub2api 菜单
nlt-services            # 服务总入口

nlt-utils gum           # 安装 gum
nlt-github-net          # GitHub 网络诊断
nlt-download            # GitHub 下载 URL 包装
nlt-port-kill list 8080
```

## 主要模块

### 开发环境

- `nlt-dev`：推荐入口，统一管理 `pip`、`uv`、Python、Go、Rust、Node.js、pnpm
- `nlt-pip-sources`：pip 镜像测速与配置
- `nlt-python-env`：基于 `uv` 创建 Python 虚拟环境

详细说明：
- [scripts/dev/README.md](scripts/dev/README.md)
- [scripts/tools/pip-sources/README.md](scripts/tools/pip-sources/README.md)
- [scripts/tools/python-env/README.md](scripts/tools/python-env/README.md)

### 服务

- `nlt-airflow`：Airflow 3 安装、启停、前台运行
- `nlt-celery`：worker / beat / flower 管理
- `nlt-paperclip`：基于官方 `npx paperclipai@latest`
- `nlt-code-server`：下载官方 standalone 包并启停
- `nlt-new-api`：下载 GitHub Releases 二进制并启停
- `nlt-sub2api`：下载 GitHub Releases 二进制并启停，保留上游 `deploy/` 文档

`sub2api` 常用命令：

```bash
nlt-sub2api install
nlt-sub2api install -v v0.1.144
nlt-sub2api list-versions
nlt-sub2api start
nlt-sub2api status
```

### 工具

- `nlt-utils`：安装 gum 与少量 shell 便利项
- `nlt-github-net`：诊断 “网页能开但 git clone 失败”
- `nlt-download`：对 GitHub 相关下载链接做可选镜像/前缀改写
- `nlt-cockpit-tools`：安装 cockpit-tools AppImage
- `nlt-port-kill`：按端口查进程并结束

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

## 前置要求

- 系统：macOS、Linux
- Shell：Bash 3.2+
- 常见依赖：`curl`

按模块补充：
- `code-server`：需要 `tar`
- `sub2api`：需要 `tar`、`gzip`，运行时依赖 PostgreSQL 15+ 与 Redis 7+
- `paperclip`：需要 Node.js 20+；公网映射需要 `socat`
- `new-api`：自动选版优先使用 `python3`

各模块的完整环境变量以对应 `setup.sh` 文件头注释为准。

## 验证与排错

运行仓库内冒烟测试：

```bash
bash tests/install_smoke.sh
bash tests/progress_smoke.sh
```

常见排错：
- 没执行权限：`chmod +x install.sh` 或对应 `setup.sh`
- PATH 未生效：手动 `export PATH="$HOME/.local/nltdeploy/bin:$PATH"`
- GitHub 下载慢或失败：优先用 `nlt-github-net`，或配合 `nlt-download`

## 相关文件

- [install.sh](install.sh)
- [scripts/dev/README.md](scripts/dev/README.md)
- [scripts/tools/pip-sources/README.md](scripts/tools/pip-sources/README.md)
- [scripts/tools/python-env/README.md](scripts/tools/python-env/README.md)
- [pyproject.toml](pyproject.toml)

## 许可证

[MIT License](LICENSE)
