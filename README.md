# fundeploy

一条命令管本机的开发环境、常驻服务、工具和镜像源。

单个静态二进制，不依赖 Python、Node 或任何运行时，装上就能跑。支持 macOS 与 Linux 的
x86_64 / arm64。

```bash
curl -fsSL https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | sh
```

## 安装

### 装机脚本

```bash
curl -fsSL https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | sh
```

默认装到 `~/.local/bin/fundeploy`。脚本不会去动你的 shell 配置，该加的 PATH 那行它会打出来。

可选参数与环境变量：

| 变量 | 作用 | 默认 |
| --- | --- | --- |
| `FUNDEPLOY_VERSION` / `--version` | 装哪一版 | `latest` |
| `FUNDEPLOY_BIN_DIR` / `--bin-dir` | 装到哪儿 | `~/.local/bin` |
| `FUNDEPLOY_RELEASE_REPO` | 从哪个仓库的 release 取 | `farfarfun/fundeploy` |
| `FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX` | GitHub 直连不通时的加速前缀 | 空 |

### Homebrew

```bash
brew install farfarfun/tap/fundeploy
```

### APT

```bash
curl -fsSL https://farfarfun.github.io/fundeploy/fundeploy.gpg | sudo tee /usr/share/keyrings/fundeploy.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/fundeploy.gpg] https://farfarfun.github.io/fundeploy stable main" | sudo tee /etc/apt/sources.list.d/fundeploy.list
sudo apt update && sudo apt install fundeploy
```

### 直接下二进制

[Releases](https://github.com/farfarfun/fundeploy/releases) 里每个平台一个 `fundeploy-<target>.tar.gz`，
解开把 `fundeploy` 丢进 PATH 即可。同目录下的 `SHA256SUMS` 可以校验。

## 升级与卸载

```bash
fundeploy upgrade            # 升到最新版
fundeploy upgrade --check    # 只看有没有新版本
fundeploy upgrade --version 0.2.0   # 装指定版本，比当前旧也照做（这是回退的用法）
```

换下来的新二进制会先跑一次 `--version` 确认能起来，再原子替换掉自己；跑不起来就原样不动。

卸载就是删掉那个文件。用 Homebrew / APT 装的交给对应包管理器：`brew uninstall fundeploy`、
`sudo apt remove fundeploy`。

## 能管什么

```bash
fundeploy list    # 列出所有能管的东西
```

### 服务 `fundeploy service`

```bash
fundeploy service                    # 列出所有服务和当前状态
fundeploy service funread-web install
fundeploy service funread-web start
fundeploy service funread-web status
```

每个服务都是这套动作：`install`、`upgrade`、`rollback <版本>`、`start`、`run`（前台）、
`stop`、`restart`、`status`、`uninstall`。

| 服务 | 说明 |
| --- | --- |
| `funfluid-web` | funfluid 前后端（18806 / 8806） |
| `funlesson-web` | funlesson 前后端（18812 / 8814） |
| `fungame-web` | fungame 后端 + 管理端 + 用户端（8809 / 8807 / 8808） |
| `funflix-web` | funflix 前后端（18810 / 8810） |
| `funread-web` | funread 前后端（18811 / 8811） |
| `funmill` | funmill 任务执行 API（8813 / 8812） |
| `new-api` | new-api 中转网关（8801） |
| `code-server` | 浏览器里的 VS Code（8080） |
| `sub2api` | sub2api 订阅转 API（8802） |
| `airflow` | Apache Airflow standalone（8815） |
| `celery` | Celery worker / beat / flower（8816） |
| `paperclip` | Paperclip 本机服务（8804） |

服务默认只监听回环地址。要对外暴露就显式设对应的 `*_HOST` / `*_ADDRESS` 环境变量。

多进程的服务（前后端一体的那几个）只提供合并命令：一条 `start` 把该起的都起来，
一条 `stop` 全停掉。换包前会先停服、装完按原状态起回来——在跑着的进程底下换全局包，
Node 还开着那棵树里的文件。

`paperclip` 额外转发官方 CLI 的动作：`install-canary`、`check`、`backup`、`channels`、
`doctor`、`health`、`onboard`、`logs`、`plugin`、`service-install`。

### 开发工具链 `fundeploy dev`

```bash
fundeploy dev                 # 列出全部的当前状态
fundeploy dev uv install
fundeploy dev go upgrade
fundeploy dev python status   # Python 多版本虚拟环境
```

覆盖 `go`、`nodejs`、`pnpm`、`rust`、`uv`，动作是 `status`、`install`、`upgrade`、
`uninstall`。`status` 会把「官方包装在哪儿」和「PATH 上实际用的是哪个」分开列——
这两者不一致是最常见的「我明明装了」。

`fundeploy dev python` 管的是 `~/opt` 下按版本号分开的 Python 虚拟环境。

### 换源 `fundeploy source`

```bash
fundeploy source              # 看全部工具当前配的是哪个源
fundeploy source pip test     # 测所有源的延迟
fundeploy source pip use      # 不给名字就自动选最快的
fundeploy source pip use tuna
fundeploy source pip reset    # 恢复官方源
fundeploy source pip add 自己的源 https://...
```

覆盖 `pip`（pip / uv / poetry / pdm 共用这份配置）、`npm`（pnpm / yarn / bun 共用）、
`brew`（配方 API、bottle 二进制、brew 仓库三处一起换）、`cargo`、`go`。

### 工具 `fundeploy tool`

`brew`（Homebrew 本身）、`cockpit-tools`（AI IDE 账号管理桌面应用）、
`open-pencil`（CLI + MCP 服务器，额外有 `install-desktop`、`mcp-config`）。

### AI 命令行 `fundeploy ai`

```bash
fundeploy ai                  # 三样的当前状态
fundeploy ai claude install
fundeploy ai all upgrade
```

覆盖 `claude`、`codex`、`cursor`。官方安装器优先，装不上再退到包管理器那条路。

### 诊断 `fundeploy doctor`

```bash
fundeploy doctor github         # 网页打得开、git clone 却卡住的那类问题
fundeploy doctor github --fix   # 按诊断结果自动挑一条通道改过去
fundeploy doctor github --undo  # 撤掉写进 git / ssh 配置里的东西
```

### 其它

```bash
fundeploy port-kill 8080 8081       # 查出谁占着端口，停掉它
fundeploy port-kill 8080 --list     # 只看不动手
fundeploy skills sync               # 同步 Gitee 上的 farfarfun-skills 仓库
```

## 开发

```bash
cargo build
cargo test
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
```

目录结构：

```text
src/
  common/    外部命令、下载、解包、平台探测等公共件
  service/   服务域：spec 声明 + 三族通用生命周期
  dev/       开发工具链
  tool/      非常驻工具
  source/    换源
  ai/        AI 命令行
  doctor/    诊断
  cli.rs     命令行解析与分发
packaging/   .deb / APT 仓库 / Homebrew formula 的生成脚本
install.sh   装机脚本
```

核心约定是**差异是数据，不是逻辑**：生命周期只写一遍，每个具体的服务/工具退化成一份
声明。加一个 PyPI + npm 的前后端服务通常只需要往 `src/service/registry.rs` 里加十几行。
装不进这套形状的（Paperclip 要直驱上游 CLI）才单独实现 trait。

发布：改 `Cargo.toml` 里的 `version`，提交，推同版本的 tag（`v0.3.0`）。CI 会核对 tag
与版本号、构建四个平台、生成校验和与安装包、发 Release，再用装机脚本真装一遍验证。

## 详细文档

- [packaging/README.md](packaging/README.md) —— 打包与发布
- [CHANGELOG.md](CHANGELOG.md)

---

## 关于 farfarfun

[farfarfun](https://github.com/farfarfun) 是一个专注于实用工具库的开源组织，
涵盖云存储、数据处理、AI、多媒体与开发工具链等方向。

- 🏠 组织主页：<https://github.com/farfarfun>
- 📧 联系：farfarfun@qq.com

本项目基于 [MIT](LICENSE) 协议开源。
