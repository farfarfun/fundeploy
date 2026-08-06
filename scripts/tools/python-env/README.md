# Python 环境自动创建脚本

## 功能概述

`setup.sh` 是一个自动化脚本，用于使用 `uv` 创建 Python 虚拟环境并安装基础包。脚本支持多个 Python 版本，提供交互式选择界面，并自动处理环境创建和包安装。

若仅需**单独安装或升级 uv 本体**，请使用 **`nltdeploy dev uv`**（实现为 `scripts/dev/uv/setup.sh`）。

## 主要特性

- ✅ **自动安装 uv**：如果系统未安装 uv，脚本会自动安装
- ✅ **多版本支持**：支持选择 Python 3.9、3.10、3.11、3.12、3.13、3.14 等多个版本
- ✅ **交互式版本选择**：运行时会弹出选择菜单，默认使用 Python 3.12
- ✅ **智能环境检测**：自动检测每个版本的环境是否已存在，显示安装时间
- ✅ **自动安装基础包**：预装 `nltbuild`、`funinstall`、`nltsecret`
- ✅ **支持额外包**：可以通过命令行参数安装额外的 Python 包
- ✅ **环境路径自动设置**：根据版本自动设置环境路径（例如：3.12 -> `~/opt/py312`）
- ✅ **错误处理和友好输出**：详细的进度信息和错误提示
- ✅ **自动激活环境**：使用 `source` 执行时，环境会自动激活

## 支持的 Python 版本

- Python 3.14
- Python 3.13
- Python 3.12（默认）
- Python 3.11
- Python 3.10
- Python 3.9

## 使用方法

### 基本使用

```bash
# 进入脚本目录
cd scripts/tools/python-env

# 给脚本添加执行权限
chmod +x setup.sh

# 方式一：直接执行（推荐用于首次安装）
./setup.sh

# 方式二：使用 source 执行（推荐，可自动激活环境）
source ./setup.sh
```

### 命令行参数

| 参数 | 简写 | 说明 | 示例 |
|------|------|------|------|
| `--version VERSION` | `-v` | 指定 Python 版本 | `-v 3.12` 或 `--version 3.11` |
| `--package PACKAGE` | `-p` | 添加额外的 Python 包（可多次使用） | `-p requests` 或 `-p numpy -p pandas` |
| `--force` | `-f` | 强制删除并重新创建现有环境 | `-f` 或 `--force` |
| `--help` | `-h` | 显示帮助信息 | `-h` 或 `--help` |

### 使用示例

```bash
# 指定 Python 版本（跳过交互式选择）
./setup.sh -v 3.12

# 指定版本并强制重新创建环境
./setup.sh --version 3.11 --force

# 指定版本并安装额外的包
./setup.sh -v 3.12 -p requests
./setup.sh -v 3.12 -p numpy -p pandas -p matplotlib

# 只安装额外的包（使用默认版本）
./setup.sh -p requests -p flask

# 非交互模式（使用默认版本）
NONINTERACTIVE=1 ./setup.sh
```

### 通过 curl 执行

```bash
# 正常执行（支持交互）
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/python-env/setup.sh | bash
# 国内（Gitee，与 GitHub 同步）
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/python-env/setup.sh | bash

# 指定版本（跳过交互）
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/python-env/setup.sh | bash -s -- -v 3.12
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/python-env/setup.sh | bash -s -- -v 3.12

# 指定版本并强制重新创建
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/python-env/setup.sh | bash -s -- -v 3.11 -f
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/python-env/setup.sh | bash -s -- -v 3.11 -f

# 指定版本并安装额外的包
curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/python-env/setup.sh | bash -s -- -v 3.12 -p requests -p flask
curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/scripts/tools/python-env/setup.sh | bash -s -- -v 3.12 -p requests -p flask
```

## 工作流程

1. **检查并安装 uv**：
   - 如果系统未安装 uv，脚本会自动下载并安装
   - 如果已安装，会显示当前版本信息

2. **选择 Python 版本**：
   - 显示交互式选择菜单，支持 3.9-3.14
   - **智能检测**：自动检测每个版本的环境是否已存在
   - **显示安装时间**：如果环境已存在，会显示安装/创建时间
   - 直接回车或输入对应数字选择版本
   - 默认版本：Python 3.12
   - 根据选择的版本自动设置环境路径

3. **创建目录**：如果 `~/opt` 目录不存在，则创建它

4. **创建虚拟环境**：
   - 如果环境已存在，会提示是否删除并重新创建
   - **默认不删除**：直接回车、输入 N 或其他任何非 y 的输入都会保留现有环境，**继续执行后续的安装包步骤**
   - **只有明确输入 y 或 Y** 才会删除旧环境并创建新环境
   - 如果环境不存在，使用 uv 创建选定版本的 Python 虚拟环境

5. **安装基础包**：依次安装 `nltbuild`、`funinstall`、`nltsecret`

6. **安装额外包**：如果通过 `-p` 参数指定了额外包，会在基础包安装完成后继续安装

7. **显示环境信息**：显示环境路径和已安装的包列表

8. **自动激活环境**：脚本执行完成后，会自动尝试激活选中的环境
   - 如果通过 `source ./setup.sh` 执行，环境会在当前 shell 中自动激活
   - 如果直接执行 `./setup.sh`，会提示如何手动激活环境

## 版本选择示例

运行脚本时会看到类似以下的选择菜单：

```
[INFO] 请选择Python版本:

  1) 3.14 [未安装]
  2) 3.13 [未安装]
  3) 3.12 [已存在] 安装时间: 2024-01-15 14:30 (默认)
  4) 3.11 [未安装]
  5) 3.10 [已存在] 安装时间: 2024-01-10 09:20
  6) 3.9 [未安装]

请选择版本 [1-6, 直接回车使用默认 3.12]:
```

**说明**：
- `[已存在]`：表示该版本的环境已经创建
- `[未安装]`：表示该版本的环境尚未创建
- `安装时间`：显示环境的创建/修改时间（如果已存在）
- 输入 `1`：选择 Python 3.14
- 输入 `2`：选择 Python 3.13
- 输入 `3` 或直接回车：选择 Python 3.12（默认）
- 输入 `4`：选择 Python 3.11
- 输入 `5`：选择 Python 3.10
- 以此类推...

## 环境路径

根据选择的 Python 版本，环境会创建在以下路径：

- Python 3.14: `~/opt/py314`
- Python 3.13: `~/opt/py313`
- Python 3.12: `~/opt/py312`
- Python 3.11: `~/opt/py311`
- Python 3.10: `~/opt/py310`
- Python 3.9: `~/opt/py39`

## 使用创建的环境

### 激活环境

**自动激活**：如果使用 `source ./setup.sh` 执行脚本，环境会在脚本完成后自动激活，无需手动激活。

**手动激活**：

```bash
# Python 3.12 示例
source ~/opt/py312/bin/activate

# 其他版本类似
source ~/opt/py311/bin/activate  # Python 3.11
source ~/opt/py310/bin/activate  # Python 3.10
```

### 停用环境

```bash
deactivate
```

### 在环境中安装其他包

激活环境后，可以使用 `uv pip install` 安装其他包：

```bash
source ~/opt/py312/bin/activate
uv pip install package_name
```

### 查看已安装的包

```bash
source ~/opt/py312/bin/activate
uv pip list
```

## 基础包说明

脚本会自动安装以下基础包：

- **nltbuild**：构建工具
- **funinstall**：卸载工具
- **nltsecret**：密钥管理工具

这些包会在创建环境后自动安装，无需手动安装。

## 环境变量

- `NONINTERACTIVE=1`：强制非交互模式，即使本地执行也使用默认值，不进行交互

## 配置说明

可以在脚本中修改以下变量来自定义设置：

- `DEFAULT_VERSION`：默认 Python 版本（默认：`3.12`）
- `PYTHON_VERSIONS`：支持的 Python 版本列表
- `PACKAGES`：要安装的基础包列表（默认：`nltbuild`、`funinstall`、`nltsecret`）

**注意**：`ENV_PATH` 会根据选择的 Python 版本自动设置，无需手动配置。

## 故障排除

### 问题：uv 自动安装失败

**可能原因**：
- 网络连接问题
- 无法访问 uv 官方安装源
- 权限问题

**解决方案**：
1. 检查网络连接
2. 手动安装 uv：`curl -LsSf https://astral.sh/uv/install.sh | sh`
3. 安装后运行：`source $HOME/.cargo/env` 或重新打开终端
4. 验证安装：`uv --version`

### 问题：包安装失败

**可能原因**：
- 网络连接问题
- 包名错误或包不存在
- PyPI 服务器不可用
- pip 源配置问题

**解决方案**：
1. 检查网络连接
2. 验证包名是否正确
3. **建议先运行 pip 源配置脚本**：`../pip-sources/setup.sh`
4. 尝试手动安装：`uv pip install package_name`

### 问题：权限错误

**解决方案**：
- 确保对 `~/opt` 目录有写权限
- 如果使用 sudo，注意环境路径可能需要调整

### 问题：环境已存在但想重新创建

**解决方案**：
- 使用 `-f` 或 `--force` 参数强制删除并重新创建
- 或在交互式提示时输入 `y` 确认删除

## 卸载环境

如果需要删除创建的环境，根据版本删除对应的目录：

```bash
# Python 3.12
rm -rf ~/opt/py312

# Python 3.11
rm -rf ~/opt/py311

# 其他版本类似
```

## 注意事项

- 脚本使用 `set -e`，遇到任何错误会立即退出
- **版本选择**：
  - 运行时会显示交互式菜单选择 Python 版本
  - 默认版本是 Python 3.12（直接回车即可）
  - 不同版本的环境会创建在不同的目录下，互不干扰
- **如果环境已存在**：
  - 脚本会询问是否删除并重新创建
  - **默认不删除**：直接回车、输入 N 或其他任何非 y 的输入都会保留现有环境，**继续执行后续的安装包步骤**
  - **只有明确输入 y 或 Y** 才会删除旧环境并创建新环境
  - 这样可以避免误删已存在的环境，同时可以在现有环境中安装或更新包
- uv 会自动安装，无需手动安装
- 所有操作都会显示详细的进度信息
- 建议在运行脚本前备份重要数据
- 可以同时创建多个不同版本的 Python 环境，它们会保存在不同的目录中

## 使用建议

### 推荐的使用流程

1. **首次使用**：
   ```bash
   # 1. 先配置 pip 源（提高后续安装速度）
   cd ../pip-sources
   ./setup.sh
   
   # 2. 创建 Python 环境
   cd ../python-env
   ./setup.sh
   ```

2. **后续使用**：
   - 如果需要创建新的 Python 版本环境，直接运行 `./setup.sh`
   - 如果需要在现有环境中安装包，运行 `./setup.sh -p package_name`

## 相关链接

- [uv 官方文档](https://github.com/astral-sh/uv)
- [Python 虚拟环境文档](https://docs.python.org/3/tutorial/venv.html)
- [pip 源配置脚本](../pip-sources/README.md)
- [项目主 README](../../README.md)
