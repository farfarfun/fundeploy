# 使用示例

## 创建 Python 环境

### 基本用法

```bash
# 创建 Python 3.12 环境
fundeploy create py312

# 创建 Python 3.11 环境
fundeploy create py311

# 创建 Python 3.10 环境
fundeploy create py310
```

### 自定义安装路径

```bash
# 安装到自定义路径
fundeploy create py312 --path /custom/path
```

### 激活环境

```bash
# Linux/macOS
source ~/opt/py312/bin/activate

# Windows PowerShell
~\opt\py312\Scripts\Activate.ps1
```

### 使用环境中的 Python

```bash
# 直接使用 Python（无需激活）
~/opt/py312/bin/python script.py
~/opt/py312/bin/pip install requests

# Windows
~\opt\py312\Scripts\python.exe script.py
~\opt\py312\Scripts\pip.exe install requests
```

## 完整工作流示例

```bash
# 1. 创建 Python 3.12 环境
fundeploy create py312

# 2. 激活环境
source ~/opt/py312/bin/activate

# 3. 安装项目依赖
pip install -r requirements.txt

# 4. 运行项目
python app.py

# 5. 退出环境
deactivate
```

## 管理多个 Python 版本

```bash
# 创建多个版本
fundeploy create py312
fundeploy create py311
fundeploy create py310

# 添加别名到 ~/.bashrc 或 ~/.zshrc
alias py312='source ~/opt/py312/bin/activate'
alias py311='source ~/opt/py311/bin/activate'
alias py310='source ~/opt/py310/bin/activate'

# 快速切换版本
py312  # 激活 Python 3.12
py311  # 激活 Python 3.11
```
