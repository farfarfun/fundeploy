# fundeploy tool download（GitHub 下载 URL 加速）

对 **`curl` 参数中的 `https://` URL** 在调用前做 **可选** 改写，仅作用于常见 GitHub 主机；未配置环境变量时与直连 `curl` 行为一致。

## 子命令

| 子命令 | 说明 |
|--------|------|
| `curl …` | 扫描参数中的 `https://…` 字符串，对命中 GitHub 白名单的项调用 `_nlt_github_download_resolve_url` 后 **`exec curl`**（退出码为 `curl` 的退出码）。 |
| `resolve-url <url>` | 打印一行改写结果（便于脚本与排障）。 |
| `install` / `update` / `reinstall` / `uninstall` | 本工具随 **fundeploy** 安装到 `libexec`；此处仅输出说明。`NONINTERACTIVE=1` 且 `install` 时会运行内置 `selftest.sh`。 |

无参（且未设置 `NONINTERACTIVE=1`）时：安装 **gum**（若缺失）后进入简单 gum 菜单。

## 环境变量与优先级

1. **`FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX`**（非空时 **最高**）：将 **完整原始 URL** 直接拼在该前缀之后（常见「代理网关」形态，例如 `https://<占位镜像域名>/https://` + `https://github.com/...`）。**占位域名可用性与合规由用户自行评估**；下文示例仅说明格式。
2. **`FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw`** 且设置了 **`FUNDEPLOY_GITHUB_RAW_MIRROR_BASE`**：仅对 **`raw.githubusercontent.com`** 将路径段拼到该前缀之后（`https://raw.githubusercontent.com/o/r/branch/file` → `<base>/o/r/branch/file`，其中 `base` 末尾斜杠可有可无）。
3. **默认**：`FUNDEPLOY_GITHUB_DOWNLOAD_MODE` 未设或 `off` 时 **不改写**。

若设置 `FUNDEPLOY_GITHUB_DOWNLOAD_MODE=hub_proxy` 却 **未** 设置 `FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX`，会向 stderr 提示并跳过改写。

发生改写时，会向 **stderr** 打印一行：`[fundeploy download] URL rewrite (…) : <原> -> <新>`。

**不会改写**：非 `https://` 的 URL、不在白名单内的主机、已是 hub 前缀开头的 URL（避免双写）。

**大文件下载进度条**：new-api / code-server 的 `install` / `update` 在已 `source` 到 `nlt-progress.sh` 时，会用 **`nlt_pb_curl_to_file`** 拉取 Release 资产（stdout 为 TTY 时显示条形进度；否则仅周期性字节日志）。

**直连提示**：未配置任何加速变量时，上述服务在下载前会向 stderr 打一行说明；不需要可设 **`FUNDEPLOY_GITHUB_DOWNLOAD_HINT=0`**。

## 识别的主机（小写比较）

- `github.com` / `www.github.com`
- `raw.githubusercontent.com`
- `api.github.com`

## 库复用

其它 Bash 脚本可：

```bash
source /path/to/scripts/lib/nlt-github-download.sh
_nlt_github_download_resolve_url "https://raw.githubusercontent.com/…"
```

（`nlt-common.sh` 也会 `source` 本库，以便 `_nlt_ensure_gum` 等路径统一走同一套 URL 改写。）

## 示例

```bash
# 关闭加速（默认）
fundeploy tool download resolve-url https://github.com/foo/bar
# → 原样输出

# 类 ghproxy：前缀 + 完整 URL（请替换为你信任且可用的网关域名）
export FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX='https://ghproxy.example/https://'
fundeploy tool download curl -fsSL https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/README.md -o /tmp/README.md

# 仅 raw 走镜像前缀（示例 base 为占位；镜像布局需与你的服务商文档一致）
export FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw
export FUNDEPLOY_GITHUB_RAW_MIRROR_BASE='https://mirror.example/gh-raw'
fundeploy tool download resolve-url https://raw.githubusercontent.com/o/r/v/file.txt
```

## 自测

仓库内：

```bash
bash scripts/tools/download/selftest.sh
```
