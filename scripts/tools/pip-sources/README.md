# pip 源自动配置脚本

## 功能概述

`setup.sh` 是一个自动化脚本，用于检测网络连通性并配置常用的 pip 镜像源。脚本会自动测试所有可用的镜像源，按下载速度排序，并生成最优的 `pip.conf` 配置。

## 主要特性

- ✅ **自动检测网络连通性**：测试所有预定义的 pip 镜像源
- ✅ **测速可分级**：默认仅测包索引页延迟（快）；可选 **`PIP_SOURCES_SPEED_TEST=1`** 做 wheel 下载速度（MB/s）
- ✅ **自动排序配置**：按下载速度自动排序，最快的源作为主源
- ✅ **保留现有配置**：自动读取本地已有的 pip 源配置，避免丢失自定义源
- ✅ **支持认证源**：支持带用户名密码的源，自动隐藏密码显示但保留完整配置
- ✅ **自动备份**：配置前自动备份现有 `pip.conf` 文件
- ✅ **友好输出**：显示详细的检测结果表格，包括延迟、下载速度等信息
- ✅ **智能过滤**：网络不可用或延迟 N/A 的源不会加入配置，但会在表格中显示

## 支持的 pip 源

### 公共镜像源

- **tsinghua** - 清华大学镜像源
- **tencent** - 腾讯云镜像源
- **ustc** - 中科大镜像源
- **bfsu** - 北京外国语大学镜像源
- **sjtu** - 上海交通大学镜像源
- **hust** - 华中科技大学镜像源
- **aliyun** - 阿里云镜像源
- **douban** - 豆瓣镜像源
- **huawei** - 华为云镜像源
- **official** - 官方源

### 内部镜像源（需要内网访问）

- **artlab-visable** - artlab-visable 内部源
- **artlab-pai** - artlab-pai 内部源
- **artlab-aop** - artlab-aop 内部源
- **tbsite** - 淘宝内部源
- **tbsite_aliyun** - 淘宝内部阿里云源
- **antfin** - 蚂蚁内部源

## 使用方法

### 基本使用

```bash
# 进入脚本目录
cd scripts/tools/pip-sources

# 给脚本添加执行权限
chmod +x setup.sh

# 运行脚本（交互式）
./setup.sh

# 非交互模式（自动配置，无需确认）
NONINTERACTIVE=1 ./setup.sh
```

### 命令行参数

| 参数 | 简写 | 说明 | 示例 |
|------|------|------|------|
| `--verbose` | `-v` | 详细模式，打印检测 URL、HTTP 状态等 | `-v` 或 `--verbose` |
| `--help` | `-h` | 显示帮助信息 | `-h` 或 `--help` |

环境变量 **`PIP_SOURCES_SPEED_TEST=1`**：启用完整 wheel 下载测速（慢）；不设时仅用索引页延迟，适合日常跑脚本。

### 通过 curl 执行

```bash
# 正常执行（支持交互）
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/pip-sources/setup.sh | bash
# 国内（Gitee，与 GitHub 同步）
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/pip-sources/setup.sh | bash

# 非交互模式
NONINTERACTIVE=1 curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/pip-sources/setup.sh | bash
NONINTERACTIVE=1 curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/pip-sources/setup.sh | bash
```

## 工作流程

1. **读取现有配置**：自动读取本地 `pip.conf` 中的现有源配置
   - 支持 `index-url` 和 `extra-index-url`（包括多行格式）
   - 自动识别带认证信息的源（如 `https://user:pass@example.com/pypi/simple/`）

2. **网络连通性检测**：测试所有预定义源和现有源的网络连通性
   - 使用 HTTP GET 请求测试连通性
   - 支持 HTTP 状态码 200-399（包括重定向）

3. **性能测试**：
   - **响应延迟测试**：请求测试包（默认 `setuptools`）的 **simple 索引页**，统计延迟（毫秒）
   - **下载速度测试（可选）**：默认 **关闭**（避免对每个源拉索引、解析 HTML、HEAD 多个 wheel，整体很慢）。需要时设置环境变量 **`PIP_SOURCES_SPEED_TEST=1`** 再运行，将按实测 **MB/s** 参与排序

4. **智能排序**：
   - 开启 **`PIP_SOURCES_SPEED_TEST=1`** 时：有实测下载速度的源优先按速度从大到小；其余按延迟排序
   - 默认仅延迟排序：数值越小越靠前
   - 网络不可用或延迟 N/A 的源不加入配置（可写入注释备忘）

5. **生成配置**：
   - 最快的源作为 `index-url`（主源）
   - 其他可用源作为 `extra-index-url`（补充源）
   - 自动配置 `trusted-host` 列表
   - 保留现有源中的认证信息

6. **确认并写入**：
   - 显示配置预览（包括所有可用源和不可用源）
   - 交互式确认后写入 `pip.conf`
   - 自动备份现有配置文件

## 检测结果示例

运行脚本后会显示类似以下的检测结果表格：

```
[INFO] 检测结果汇总:

序号 源标识          状态       延迟       下载速度 源名称
------ ------------------ ------------ ------------ ------------ ------------------------------
1      tbsite             ✓ 可用   254ms        4.09MB/s     淘宝内部源
2      aliyun             ✓ 可用   131ms        2.81MB/s     阿里云镜像源
3      antfin             ✓ 可用   147ms        2.78MB/s     蚂蚁内部源
4      sjtu               ✓ 可用   191ms        1.90MB/s     上海交通大学镜像源
5      official           ✓ 可用   291ms        1.86MB/s     官方源
6      artlab-visable     ✓ 可用   205ms        N/A          artlab-visable
7      artlab-pai         ✓ 可用   230ms        N/A          artlab-pai
8      douban             ✗ 不可用 N/A          N/A          豆瓣镜像源
```

**说明**：
- `✓ 可用`：源可用，会被加入配置
- `✗ 不可用`：源不可用，只显示在表格中，不加入配置
- `延迟`：HTTP 响应延迟（毫秒）
- `下载速度`：实际下载速度（MB/s），N/A 表示无法测试下载速度但源可响应

## 配置说明

### 配置文件位置

脚本会自动检测并配置以下位置的 `pip.conf`：

- **macOS/Linux**: `~/.pip/pip.conf` 或 `~/.config/pip/pip.conf`
- **Windows**: `%APPDATA%\pip\pip.ini`

### 配置格式示例

生成的 `pip.conf` 格式示例：

```ini
[global]
index-url = https://mirrors.aliyun.com/pypi/simple/
extra-index-url = 
    https://mirrors.cloud.tencent.com/pypi/simple/
    https://pypi.mirrors.ustc.edu.cn/simple/
trusted-host = 
    mirrors.aliyun.com
    mirrors.cloud.tencent.com
    pypi.mirrors.ustc.edu.cn
```

### 保留现有配置

脚本会自动读取本地已有的 `pip.conf` 配置，包括：

- `index-url` 配置
- `extra-index-url` 配置（支持多行格式）
- 带认证信息的源（如 `https://user:pass@example.com/pypi/simple/`）

这些现有源会被：

- ✅ 自动添加到检测列表
- ✅ 优先进行测试
- ✅ 如果可用，会保留在最终配置中
- ✅ 显示时会自动隐藏密码（如 `user:***@example.com`）

### 备份机制

脚本在写入新配置前会自动备份现有配置文件：

- 备份文件名格式：`pip.conf.backup.YYYYMMDD_HHMMSS`
- 备份位置：与配置文件相同目录

## 环境变量

- `NONINTERACTIVE=1`：强制非交互模式，自动配置无需确认
- `PIP_SOURCES_TEST_TIMEOUT`：单次 HTTP 总超时（秒），默认 `5`；慢网可设为 `10`
- `PIP_SOURCES_CONNECT_TIMEOUT`：连接阶段超时（秒），默认 `2`
- `PIP_SOURCES_PARALLEL_JOBS`：并行探测源数量，默认 `8`；设为 `1` 则完全顺序执行（便于排错）。开启 `PIP_SOURCES_SPEED_TEST=1` 时脚本会强制顺序，避免多路抢带宽
- `PIP_SOURCES_SPEED_TEST=1`：见上文，启用 wheel 实测（慢）

## 故障排除

### 问题：所有源都不可用

**可能原因**：
- 网络连接问题
- 防火墙阻止访问

**解决方案**：
1. 检查网络连接
2. 检查防火墙设置
3. 尝试手动访问某个镜像源
4. 使用 `-v` 参数查看详细检测信息

### 问题：配置写入失败

**可能原因**：
- 权限不足
- 目录不存在

**解决方案**：
1. 确保对配置目录有写权限
2. 手动创建配置目录：
   ```bash
   mkdir -p ~/.pip
   # 或
   mkdir -p ~/.config/pip
   ```

### 问题：检测速度慢

**可能原因**：
- 网络延迟高或部分源不可达（过去会长时间等满超时）
- 开启了 `PIP_SOURCES_SPEED_TEST=1`（wheel 实测很慢）

**说明与建议**：
1. 默认已对 curl 使用较短的**总超时**与**连接超时**，并对多源 **并行探测**（默认最多 8 路），整体比旧版顺序 + 10s 超时快很多
2. 若仍偏慢：可适当减小 `PIP_SOURCES_TEST_TIMEOUT`，或把 `PIP_SOURCES_PARALLEL_JOBS` 调大（注意带宽与远端限流）
3. 排错时用 `PIP_SOURCES_PARALLEL_JOBS=1` 与 `-v` 查看顺序输出
4. 日常勿开 `PIP_SOURCES_SPEED_TEST`，仅用索引延迟即可排序

## 技术细节

### 网络检测方法

- 使用 `curl` 进行 HTTP 请求测试
- 支持 HTTP 状态码 200-399（包括重定向）
- 自动处理 URL 格式（确保以 `/simple/` 结尾）

### 速度测试方法

1. **延迟测试**：测试访问包索引页面的 HTTP 响应时间
2. **下载速度测试**：
   - 优先尝试下载小型的 `.whl` 文件（< 1MB）
   - 如果找不到 `.whl` 文件，则下载包索引页面本身
   - 计算实际下载速度（MB/s）

### 排序算法

- 有下载速度的源：使用 `(1000 - speed) * 1000` 作为排序键，速度越大排序键越小，排在前面
- 无下载速度的源：使用 `2000000 + latency` 作为排序键，确保排在有速度的源后面
- 使用数值比较确保排序正确

## 相关链接

- [pip 配置文档](https://pip.pypa.io/en/stable/topics/configuration/)
- [项目主 README](../README.md)
