# Python 环境使用示例

Python 环境由 `fundeploy dev python` 管理。环境默认创建在 `~/opt/py<版本号>`，例如 Python 3.12 对应 `~/opt/py312`。

## 创建环境

```bash
# 创建 Python 3.12 环境
fundeploy dev python install -v 3.12

# 创建 Python 3.11 环境，并额外安装包
fundeploy dev python install -v 3.11 -p requests -p flask
```

`install` 不带 `-v` 时会进入版本选择；非交互使用默认版本：

```bash
NONINTERACTIVE=1 fundeploy dev python install
```

## 使用环境

```bash
# 激活环境
source ~/opt/py312/bin/activate

# 或直接使用环境中的解释器
~/opt/py312/bin/python script.py
~/opt/py312/bin/pip install requests
```

## 管理环境

```bash
# 更新默认环境中的预装包
fundeploy dev python update

# 删除并重建默认环境
fundeploy dev python reinstall

# 删除默认环境（会要求确认）
fundeploy dev python uninstall
```

可用的版本和附加包参数见：

```bash
fundeploy dev python --help
```
