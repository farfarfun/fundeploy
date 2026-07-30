#!/bin/bash

# 自动创建Python环境并安装基础包的脚本
# 使用uv创建Python 3.12环境

# 原为裸 set -e：缺 -u 会让拼错的变量静默展开为空，缺 pipefail 会让管道中
# 非最后一环的失败被吞掉。本文件大量使用 local x=$(cmd) 与管道，两者都需要。
set -euo pipefail

_PSDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${_PSDIR}/../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_PSDIR}/../lib" && pwd)"
elif [[ -f "${_PSDIR}/../../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_PSDIR}/../../lib" && pwd)"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${_PSDIR}/../lib 与 ${_PSDIR}/../../lib）" >&2
  exit 1
fi
# shellcheck source=../lib/nlt-common.sh
source "${_NLT_LIB}/nlt-common.sh"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 配置变量
PYTHON_VERSION=""
ENV_PATH=""
DEFAULT_VERSION="3.12"
# 支持的Python版本列表（从高到低排序）
PYTHON_VERSIONS=("3.14" "3.13" "3.12" "3.11" "3.10" "3.9" "3.8")
PACKAGES=("funbuild" "funinstall" "funsecret")

# 检测是否可以交互（在脚本开始时检测一次）
# 优先级：环境变量 > 实际检测
# 可以通过设置 NONINTERACTIVE=1 强制非交互模式
# 关键：即使通过管道执行，如果 /dev/tty 可用，也可以交互
if [ "${NONINTERACTIVE:-}" = "1" ]; then
    IS_INTERACTIVE=false
elif { exec 3</dev/tty; } 2>/dev/null; then
    # 真正打开 /dev/tty 才算可交互。原实现用 [ -c/-r/-w /dev/tty ]，那是对
    # 路径的 access(2) 检查；/dev/tty 权限为 0666，在 cron/systemd/docker
    # （无 -t）下也会通过，于是 IS_INTERACTIVE=true，之后 read < /dev/tty
    # 打开失败，set -e 让脚本以 "cannot open /dev/tty" 中止，而不是回退默认值。
    exec 3<&-
    IS_INTERACTIVE=true
elif [ -t 0 ] && [ -t 1 ]; then
    # stdin 和 stdout 都是终端
    IS_INTERACTIVE=true
else
    # 无法交互
    IS_INTERACTIVE=false
fi

# 打印带颜色的消息
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}


# 检查uv是否安装，不存在则自动安装
check_uv() {
    if ! command -v uv &> /dev/null; then
        print_warn "uv 未安装，开始自动安装..."
        print_info "正在从官方源安装 uv..."
        
        # 使用官方安装脚本安装 uv
        if curl -LsSf --proto '=https' --proto-redir '=https' --tlsv1.2 \
             https://astral.sh/uv/install.sh | sh; then
            # 将 uv 添加到 PATH（通常安装在 ~/.cargo/bin）
            if [ -f "$HOME/.cargo/env" ]; then
                source "$HOME/.cargo/env"
            fi
            
            # 验证安装是否成功
            if command -v uv &> /dev/null; then
                print_info "uv 安装成功: $(uv --version)"
            else
                print_error "uv 安装后仍无法找到，请手动添加到 PATH"
                print_info "请运行: source $HOME/.cargo/env"
                exit 1
            fi
        else
            print_error "uv 安装失败，请手动安装"
            print_info "安装方法：curl -LsSf https://astral.sh/uv/install.sh | sh"
            exit 1
        fi
    else
        print_info "检测到 uv: $(uv --version)"
    fi
}

# 检查环境是否存在并获取安装时间
check_env_status() {
    local version=$1
    local version_num=$(echo "$version" | tr -d '.')
    local env_path="$HOME/opt/py${version_num}"
    
    if [ -d "$env_path" ]; then
        # 获取环境目录的修改时间
        local env_time=""
        
        # 尝试使用 stat 命令（macOS 和 Linux 格式不同）
        if [[ "$OSTYPE" == "darwin"* ]]; then
            # macOS
            env_time=$(stat -f "%Sm" -t "%Y-%m-%d %H:%M" "$env_path" 2>/dev/null)
        else
            # Linux
            env_time=$(stat -c "%y" "$env_path" 2>/dev/null | awk '{print $1" "$2}' | cut -d':' -f1,2)
        fi
        
        # 如果 stat 失败，尝试使用 date 命令
        if [ -z "$env_time" ]; then
            local timestamp=$(stat -f "%m" "$env_path" 2>/dev/null || stat -c "%Y" "$env_path" 2>/dev/null)
            if [ -n "$timestamp" ]; then
                env_time=$(date -r "$timestamp" "+%Y-%m-%d %H:%M" 2>/dev/null || date -d "@$timestamp" "+%Y-%m-%d %H:%M" 2>/dev/null)
            fi
        fi
        
        if [ -n "$env_time" ]; then
            echo "[已存在] 安装时间: $env_time"
        else
            echo "[已存在]"
        fi
    else
        echo "[未安装]"
    fi
}

# 选择Python版本
select_python_version() {
    print_info "请选择Python版本:"
    echo ""
    
    # 显示版本列表，包含环境状态和安装时间
    local count=${#PYTHON_VERSIONS[@]}
    local index=1
    for ver in "${PYTHON_VERSIONS[@]}"; do
        local status=$(check_env_status "$ver")
        local default_mark=""
        if [ "$ver" = "$DEFAULT_VERSION" ]; then
            default_mark=" (默认)"
        fi
        printf "  %d) %s %s%s\n" "$index" "$ver" "$status" "$default_mark"
        ((index++))
    done
    echo ""
    
    # 检查是否可以交互
    if [ "$IS_INTERACTIVE" = "false" ]; then
        # 无法交互，自动使用默认版本
        print_info "无法进行交互式选择，自动使用默认版本: Python $DEFAULT_VERSION"
        PYTHON_VERSION="$DEFAULT_VERSION"
        VERSION_NUM=$(echo "$DEFAULT_VERSION" | tr -d '.')
        ENV_PATH="$HOME/opt/py${VERSION_NUM}"
        print_info "环境路径: $ENV_PATH"
    else
        # 可以交互（包括通过 curl 执行但 /dev/tty 可用的情况），使用read从/dev/tty读取输入
        while true; do
            read -r -p "请选择版本 [1-$count, 直接回车使用默认 $DEFAULT_VERSION]: " choice < /dev/tty
            
            # 如果直接回车，使用默认版本
            if [ -z "$choice" ] || [ "$choice" = "" ]; then
                PYTHON_VERSION="$DEFAULT_VERSION"
                VERSION_NUM=$(echo "$DEFAULT_VERSION" | tr -d '.')
                ENV_PATH="$HOME/opt/py${VERSION_NUM}"
                print_info "使用默认版本: Python $PYTHON_VERSION"
                print_info "环境路径: $ENV_PATH"
                break
            fi
            
            # 检查输入是否为有效数字
            if [[ "$choice" =~ ^[0-9]+$ ]]; then
                if [ "$choice" -ge 1 ] && [ "$choice" -le "$count" ]; then
                    # 数组索引从0开始，所以需要减1
                    local selected_index=$((choice - 1))
                    PYTHON_VERSION="${PYTHON_VERSIONS[$selected_index]}"
                    VERSION_NUM=$(echo "$PYTHON_VERSION" | tr -d '.')
                    ENV_PATH="$HOME/opt/py${VERSION_NUM}"
                    print_info "已选择 Python $PYTHON_VERSION"
                    print_info "环境路径: $ENV_PATH"
                    break
                else
                    print_error "无效选择 '$choice'，请输入 1-$count 之间的数字"
                    echo ""
                fi
            else
                print_error "无效输入 '$choice'，请输入数字 1-$count"
                echo ""
            fi
        done
    fi
    
    echo ""
}

# 创建目录
create_directory() {
    if [ ! -d "$HOME/opt" ]; then
        print_info "创建目录: $HOME/opt"
        mkdir -p "$HOME/opt"
    fi
}

# 创建Python环境
create_venv() {
    if [ -d "$ENV_PATH" ]; then
        print_warn "Python环境已存在: $ENV_PATH"
        
        # 检查是否可以交互
        if [ "$IS_INTERACTIVE" = "false" ]; then
            # 无法交互，默认不删除，继续安装包
            print_info "无法进行交互式选择，保留现有环境，继续安装包..."
            return 0
        else
            # 可以交互（包括通过 curl 执行但 /dev/tty 可用的情况），询问用户
            read -p "是否要删除并重新创建? [y/N] (直接回车默认不删除，继续安装包): " -n 1 -r < /dev/tty
            echo
        fi
        
        # 只有明确输入 y 或 Y 时才删除，其他情况（包括回车、空输入、N等）都默认不删除，继续执行
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            print_info "删除现有环境..."
            rm -rf "$ENV_PATH"
            print_info "正在创建Python ${PYTHON_VERSION}环境: $ENV_PATH"
            # set -e 下 uv venv 失败会直接终止脚本，`if [ $? -eq 0 ]` 永远为真，
            # else 分支不可达、报错信息从不打印。改为直接判断命令本身。
            if uv venv "$ENV_PATH" --python "$PYTHON_VERSION"; then
                print_info "Python环境创建成功"
            else
                print_error "Python环境创建失败"
                exit 1
            fi
        else
            print_info "保留现有环境，继续安装包..."
            return 0
        fi
    else
        print_info "正在创建Python ${PYTHON_VERSION}环境: $ENV_PATH"
        if uv venv "$ENV_PATH" --python "$PYTHON_VERSION"; then
            print_info "Python环境创建成功"
        else
            print_error "Python环境创建失败"
            exit 1
        fi
    fi
}

# 安装包
install_packages() {
    print_info "开始安装基础包..."
    
    # 激活虚拟环境并安装包
    source "$ENV_PATH/bin/activate"
    
    # 使用uv pip安装包
    for package in "${PACKAGES[@]}"; do
        print_info "正在安装: $package"
        if uv pip install -U "$package"; then
            print_info "$package 安装成功"
        else
            print_error "$package 安装失败"
            exit 1
        fi
    done
    
    print_info "所有基础包安装完成"
}

# 显示环境信息
show_info() {
    print_info "环境设置完成！"
    echo ""
    echo "环境路径: $ENV_PATH"
    echo "Python版本: $PYTHON_VERSION"
    source "$ENV_PATH/bin/activate"
    #echo "已安装的包:"
    #uv pip list
    echo ""
}

# 激活环境
activate_environment() {
    print_info "正在激活环境..."
    
    # 尝试激活环境
    if [ -f "$ENV_PATH/bin/activate" ]; then
        source "$ENV_PATH/bin/activate"
        
        # 检查是否激活成功（通过检查 VIRTUAL_ENV 变量）
        if [ -n "$VIRTUAL_ENV" ] && [ "$VIRTUAL_ENV" = "$ENV_PATH" ]; then
            print_info "环境已激活！"
            echo ""
            print_info "当前Python版本: $(python --version 2>&1)"
            print_info "当前环境路径: $VIRTUAL_ENV"
            echo ""
        else
            # 如果通过直接执行脚本，无法在当前shell中激活
            print_warn "注意：脚本是直接执行的，环境无法在当前shell中自动激活"
            print_info "要激活环境，请运行以下命令："
            echo ""
            echo "  source $ENV_PATH/bin/activate"
            echo ""
            print_info "或者使用以下命令执行脚本以自动激活："
            echo ""
            echo "  source ./setup.sh"
            echo ""
        fi
    else
        print_error "激活脚本不存在: $ENV_PATH/bin/activate"
    fi
}

# 更新已存在环境中的基础包（默认版本路径；可通过 PYTHON_VERSION 覆盖）
cmd_pyenv_update() {
    check_uv
    local ver="${PYTHON_VERSION:-$DEFAULT_VERSION}"
    local num
    num=$(echo "$ver" | tr -d '.')
    ENV_PATH="$HOME/opt/py${num}"
    if [ ! -x "$ENV_PATH/bin/python" ]; then
        print_error "未找到环境 $ENV_PATH，请先 install 或设置 PYTHON_VERSION"
        exit 1
    fi
    print_info "更新 $ENV_PATH 内基础包..."
    for package in "${PACKAGES[@]}"; do
        uv pip install --python "$ENV_PATH/bin/python" -U "$package" || exit 1
    done
    print_info "update 完成。"
}

# 删除并重建默认版本环境
cmd_pyenv_reinstall() {
    # 不再为一次是非题去装 gum：nlt_ui_confirm 无 gum 时降级为 read y/N。
    nlt_confirm_destructive "将删除并重建 Python ${DEFAULT_VERSION} 环境（~/opt/py*），继续？" \
        PYTHON_ENV_ASSUME_YES || return 1
    PYTHON_VERSION="$DEFAULT_VERSION"
    local num
    num=$(echo "$PYTHON_VERSION" | tr -d '.')
    ENV_PATH="$HOME/opt/py${num}"
    rm -rf "$ENV_PATH"
    check_uv
    create_directory
    print_info "正在创建 Python ${PYTHON_VERSION} 环境: $ENV_PATH"
    uv venv "$ENV_PATH" --python "$PYTHON_VERSION" || exit 1
    for package in "${PACKAGES[@]}"; do
        print_info "正在安装: $package"
        uv pip install --python "$ENV_PATH/bin/python" "$package" || exit 1
    done
    print_info "reinstall 完成: $ENV_PATH"
}

# 删除所选版本环境（交互选版本；NONINTERACTIVE=1 时删默认版本）
cmd_pyenv_uninstall() {
    _nlt_ensure_gum || exit 1
    if [ "${NONINTERACTIVE:-}" = "1" ]; then
        PYTHON_VERSION="$DEFAULT_VERSION"
        local num
        num=$(echo "$PYTHON_VERSION" | tr -d '.')
        ENV_PATH="$HOME/opt/py${num}"
    else
        select_python_version
    fi
    nlt_confirm_destructive "永久删除 $ENV_PATH？" PYTHON_ENV_ASSUME_YES || return 1
    nlt_safe_rm "$ENV_PATH" || return 1
    print_info "已删除 $ENV_PATH"
}

nlt_cli_main() {
    if [ $# -eq 0 ]; then
        _nlt_ensure_gum || exit 1
        local pick
        pick=$(gum choose --header "Python 环境 (uv)" \
            "install" "update" "reinstall" "uninstall" "help") || exit 0
        [ -z "$pick" ] && exit 0
        [ "$pick" = "help" ] && { print_info "子命令: install | update | reinstall | uninstall；环境变量见脚本头部"; exit 0; }
        set -- "$pick"
    fi
    case "$1" in
        install)
            shift
            main "$@"
            ;;
        update|upgrade)
            cmd_pyenv_update
            ;;
        reinstall)
            cmd_pyenv_reinstall
            ;;
        uninstall)
            cmd_pyenv_uninstall
            ;;
        help | -h | --help)
            print_info "子命令: install | update | reinstall | uninstall"
            exit 0
            ;;
        *)
            main "$@"
            ;;
    esac
}

# 主函数
main() {
    _nlt_ensure_gum || exit 1
    print_info "开始设置Python环境..."
    echo ""
    
    check_uv
    select_python_version
    create_directory
    create_venv
    install_packages
    show_info
    
    # 自动激活环境
    activate_environment
    
    print_info "脚本执行完成！"
}

# 执行主函数（无参时 gum 选子命令）
nlt_cli_main "$@"
