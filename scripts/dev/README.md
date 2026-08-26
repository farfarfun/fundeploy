# 开发工具统一入口（`scripts/dev`）

本目录是 **fundeploy 的开发环境实现**：pip、**uv 本体**、Python（基于 uv 的虚拟环境）与多语言工具链的安装、升级说明与脚本集中在此。

**推荐顺序**：`fundeploy dev pip`（镜像）→ **`fundeploy dev uv`**（安装/升级 Astral **uv**）→ `fundeploy dev python`（用 uv 建 venv）。仅建环境、不关心单独升级 uv 时，可直接 `fundeploy dev python`（内部仍会按需自动安装 uv）。

## 推荐用法（已安装到 PATH）

一键安装仓库脚本后，优先使用：

```bash
fundeploy dev              # 有 gum 时弹出菜单；否则打印用法
fundeploy dev pip          # 委派到 pip 源 / 镜像配置
fundeploy dev uv           # 安装：官方 install.sh；升级：`fundeploy dev uv update`
fundeploy dev python       # 委派到 uv + Python 虚拟环境
fundeploy dev go           # 官方 tarball 安装到 GO_INSTALL_ROOT（默认 ~/opt/go）
fundeploy dev rust         # rustup 非交互安装 / 升级 stable
fundeploy dev nodejs       # Node.js 官方预编译包到 NODE_INSTALL_ROOT（默认 ~/opt/node）
fundeploy dev pnpm         # 在已有 Node 前提下用 corepack 启用 pnpm
```

各子目录 `*/setup.sh` 也可单独执行（与仓库内其它 `setup.sh` 约定一致）。

## uv（Astral）

- **内部脚本**：`scripts/dev/uv/setup.sh`，通过 `fundeploy dev uv …` 调用。
- **安装**：`fundeploy dev uv` 或 `fundeploy dev uv install` — 管道执行官方 `https://astral.sh/uv/install.sh`（可用 `UV_INSTALL_URL` 覆盖镜像/内网副本地址）。
- **升级**：`fundeploy dev uv update` — 若 PATH 中已有 `uv`，执行 **`uv self update`**；否则再次走官方安装脚本。
- **与 python-env**：`python-env/setup.sh` 在创建环境前仍会 **按需自动** `curl … | sh` 安装 uv。

## 环境变量速查

| 变量 | 用途 | 默认 |
|------|------|------|
| `UV_INSTALL_DIR` | 官方安装器：二进制安装目录 | 由官方脚本决定（常见 `~/.local/bin`） |
| `INSTALLER_NO_MODIFY_PATH` | 设为 `1` 时安装器不自动改 shell 配置 | 未设 |
| `UV_INSTALL_URL` | 覆盖 uv 安装脚本 URL | `https://astral.sh/uv/install.sh` |
| `GO_INSTALL_ROOT` | Go 解压目标（GOROOT） | `~/opt/go` |
| `GO_VERSION` | 强制版本，如 `go1.22.4`；不设则从 go.dev 读取 | 自动 |
| `RUSTUP_HOME` / `CARGO_HOME` | rustup 数据目录 | rustup 默认 |
| `NODE_VERSION` | Node 版本号，如 `22.14.0` | `22.14.0` |
| `NODE_INSTALL_ROOT` | Node 安装根（内含 `bin/node`） | `~/opt/node` |
| `PNPM_USE_NPM_GLOBAL` | 设为 `1` 时用 `npm install -g pnpm` 代替 corepack | 未设 |

## 迁移说明

- **以前**：文档主路径常写「先装 `nlt-pip-sources` 再装 `nlt-python-env`」。
- **现在**：统一使用 `fundeploy dev pip|uv|python`；旧命令入口不再安装。

## 相关文档

- [pip-sources 说明](../tools/pip-sources/README.md)
- [python-env 说明](../tools/python-env/README.md)
