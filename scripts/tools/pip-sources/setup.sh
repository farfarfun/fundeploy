#!/usr/bin/env bash

# 自动配置 pip 源的脚本
# 检测网络连通性并配置常用的 pip 镜像源
#
# 须用 bash 运行；若使用「sh setup.sh」，POSIX sh 无进程替换等特性，会自动改由 bash 重新执行。

if [ -z "${BASH_VERSION:-}" ]; then
  exec /usr/bin/env bash "$0" "$@"
fi

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
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 配置变量
PIP_CONFIG_DIR=""
PIP_CONFIG_FILE=""
BACKUP_SUFFIX=".backup.$(date +%Y%m%d_%H%M%S)"

# 统一的 pip 镜像源配置（返回 URL|显示名称）
get_pip_source_info() {
    case "$1" in
        tsinghua) echo "https://pypi.tuna.tsinghua.edu.cn/simple/|清华大学镜像源" ;;
        aliyun) echo "https://mirrors.aliyun.com/pypi/simple/|阿里云镜像源" ;;
        douban) echo "https://pypi.douban.com/simple/|豆瓣镜像源" ;;
        tencent) echo "https://mirrors.cloud.tencent.com/pypi/simple/|腾讯云镜像源" ;;
        huawei) echo "https://mirrors.huaweicloud.com/repository/pypi/simple|华为云镜像源" ;;
        ustc) echo "https://pypi.mirrors.ustc.edu.cn/simple/|中科大镜像源" ;;
        bfsu) echo "https://mirrors.bfsu.edu.cn/pypi/web/simple/|北京外国语大学镜像源" ;;
        sjtu) echo "https://mirror.sjtu.edu.cn/pypi/web/simple/|上海交通大学镜像源" ;;
        hust) echo "http://pypi.hustunique.com/|华中科技大学镜像源" ;;
        artlab-visable) echo "https://artlab.alibaba-inc.com/1/pypi/visable|artlab-visable" ;;
        artlab-pai) echo "https://artlab.alibaba-inc.com/1/pypi/pai|artlab-pai" ;;
        artlab-aop) echo "https://artlab.alibaba-inc.com/1/pypi/aop|artlab-aop" ;;
        tbsite) echo "http://yum.tbsite.net/pypi/simple|淘宝内部源" ;;
        tbsite_aliyun) echo "http://yum.tbsite.net/aliyun-pypi/simple|淘宝内部阿里云源" ;;
        antfin) echo "https://pypi.antfin-inc.com/simple|蚂蚁内部源" ;;
        official) echo "https://pypi.org/simple|官方源" ;;
        
        *) echo "|" ;;
    esac
}

# 该源是否走明文 HTTP（即需要 trusted-host、且流量可被篡改）。
_pip_source_is_insecure() {
    local url
    url="$(get_pip_source_url "$1")"
    [[ "$url" == http://* ]]
}

# 与 get_pip_source_info 中预定义源顺序一致（避免 sed 解析函数体）
PIP_PREDEFINED_SOURCES=(
    tsinghua aliyun douban tencent huawei ustc bfsu sjtu hust
    artlab-visable artlab-pai artlab-aop tbsite tbsite_aliyun antfin official
)

# 根据源根 URL 构造「包索引页」URL（供延迟/测速共用）
build_package_index_url() {
    local source_url="$1"
    local package_name="$2"
    local base_url package_url

    if [[ "$source_url" =~ /simple/?$ ]] || [[ "$source_url" =~ /simple/ ]]; then
        base_url="${source_url%/}"
        [[ "$base_url" =~ /simple/?$ ]] || base_url="${base_url}/simple"
        base_url="${base_url%/}/"
        package_url="${base_url}${package_name}/"
    elif [[ "$source_url" =~ /pypi/ ]]; then
        package_url="${source_url%/}/${package_name}/"
    elif [[ "$source_url" =~ /[^/]+/[^/]+ ]] && [[ ! "$source_url" =~ /simple/?$ ]]; then
        package_url="${source_url%/}/${package_name}/"
    else
        package_url="${source_url%/}/simple/${package_name}/"
    fi
    printf '%s\n' "$package_url"
}

# 获取源的 URL（支持自定义源）
get_pip_source_url() {
    local source_name=$1
    
    # 检查是否是自定义源
    if [[ "$source_name" =~ ^custom- ]]; then
        # 从 CUSTOM_SOURCES 中查找
        for custom_item in "${CUSTOM_SOURCES[@]}"; do
            local source_id=$(echo "$custom_item" | LC_ALL=C cut -d'|' -f1)
            if [ "$source_id" = "$source_name" ]; then
                echo "$custom_item" | LC_ALL=C cut -d'|' -f2
                return 0
            fi
        done
        echo ""
        return 1
    fi
    
    # 否则使用预定义源
    local info=$(get_pip_source_info "$source_name")
    echo "$info" | LC_ALL=C cut -d'|' -f1
}

# 获取源的显示名称（支持自定义源）
get_source_display_name() {
    local source_name=$1
    
    # 检查是否是自定义源
    if [[ "$source_name" =~ ^custom- ]]; then
        # 从 CUSTOM_SOURCES 中查找
        for custom_item in "${CUSTOM_SOURCES[@]}"; do
            local source_id=$(echo "$custom_item" | LC_ALL=C cut -d'|' -f1)
            if [ "$source_id" = "$source_name" ]; then
                echo "$custom_item" | LC_ALL=C cut -d'|' -f3
                return 0
            fi
        done
        echo "$source_name"
        return 1
    fi
    
    # 否则使用预定义源
    local info=$(get_pip_source_info "$source_name")
    echo "$info" | LC_ALL=C cut -d'|' -f2
}

PIP_SOURCE_NAMES=("${PIP_PREDEFINED_SOURCES[@]}")
CUSTOM_SOURCES=()

# 命令行参数
SELECTED_SOURCE="${SELECTED_SOURCE:-}"
SHOW_HELP=false
# 整次请求上限（秒）；慢网或不稳定可设 PIP_SOURCES_TEST_TIMEOUT=10
TEST_TIMEOUT="${PIP_SOURCES_TEST_TIMEOUT:-5}"
# TCP/TLS 连接阶段上限（秒）；不可达主机尽快失败，避免多源顺序探测拖很久
CONNECT_TIMEOUT="${PIP_SOURCES_CONNECT_TIMEOUT:-2}"
VERBOSE=false    # 详细模式（-v / --verbose 打开）
TEST_PACKAGE="setuptools"  # 用于延迟/测速的包索引页（常用、索引小）
TEST_WHEEL_SIZE=0  # 保留兼容，未使用
# 设为 1 时对每个源做 wheel 下载速度测试（慢）；默认仅 HTTP 延迟，足够排序
PIP_SOURCES_SPEED_TEST="${PIP_SOURCES_SPEED_TEST:-0}"
# 并行探测源数量（默认 8）；设为 1 完全顺序。wheel 实测时强制为 1，避免多路抢带宽
PIP_SOURCES_PARALLEL_JOBS="${PIP_SOURCES_PARALLEL_JOBS:-8}"

# 检测是否可以交互
if [ "${NONINTERACTIVE:-}" = "1" ]; then
    IS_INTERACTIVE=false
elif [ -c /dev/tty ] 2>/dev/null && [ -r /dev/tty ] && [ -w /dev/tty ]; then
    IS_INTERACTIVE=true
elif [ -t 0 ] && [ -t 1 ]; then
    IS_INTERACTIVE=true
else
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

print_debug() {
    echo -e "${BLUE}[DEBUG]${NC} $1" >&2
}

# 显示帮助信息
show_help() {
    cat << EOF
用法: $0 [选项]

选项:
  -v, --verbose          详细模式，打印每条检测 URL / HTTP 码等
  -h, --help             显示此帮助信息

环境变量:
  PIP_SOURCES_SPEED_TEST=1       对每个源做 wheel 下载测速（慢）；默认仅测延迟
  PIP_SOURCES_TEST_TIMEOUT=秒数    单次 curl 总超时，默认 5；慢网可加大
  PIP_SOURCES_CONNECT_TIMEOUT=秒数 连接阶段超时，默认 2
  PIP_SOURCES_PARALLEL_JOBS=N      并行探测源个数，默认 8；设为 1 则顺序执行
                                   （PIP_SOURCES_SPEED_TEST=1 时始终顺序）

说明:
  脚本会检测各 pip 源连通性，默认按包索引页延迟排序；可选开启完整下载测速。
  能响应的源会写入配置，不可用源写入注释备忘。

支持的 pip 源:
  tsinghua      - 清华大学镜像源 (https://pypi.tuna.tsinghua.edu.cn/simple/)
  tencent       - 腾讯云镜像源 (https://mirrors.cloud.tencent.com/pypi/simple/)
  ustc          - 中科大镜像源 (https://pypi.mirrors.ustc.edu.cn/simple/)
  bfsu          - 北京外国语大学镜像源 (https://mirrors.bfsu.edu.cn/pypi/web/simple/)
  sjtu          - 上海交通大学镜像源 (https://mirror.sjtu.edu.cn/pypi/web/simple/)
  hust          - 华中科技大学镜像源 (http://pypi.hustunique.com/)
  artlab-visable - artlab-visable (https://artlab.alibaba-inc.com/1/pypi/visable)
  artlab-pai     - artlab-pai (https://artlab.alibaba-inc.com/1/pypi/pai)
  artlab-aop     - artlab-aop (https://artlab.alibaba-inc.com/1/pypi/aop)
  tbsite        - 淘宝内部源 (http://yum.tbsite.net/pypi/simple)
  tbsite_aliyun - 淘宝内部阿里云源 (http://yum.tbsite.net/aliyun-pypi/simple)
  antfin        - 蚂蚁内部源 (https://pypi.antfin-inc.com/simple)
  aliyun        - 阿里云镜像源 (https://mirrors.aliyun.com/pypi/simple/)
  douban        - 豆瓣镜像源 (https://pypi.douban.com/simple/)
  huawei        - 华为云镜像源 (https://mirrors.huaweicloud.com/repository/pypi/simple)
  official      - 官方源 (https://pypi.org/simple)

示例:
  $0                      # 自动检测并配置（默认按延迟排序，较快）
  PIP_SOURCES_SPEED_TEST=1 $0   # 按实际下载速度排序（较慢）
  $0 -v                   # 详细调试输出
  NONINTERACTIVE=1 $0     # 非交互（gum 菜单仍可能拉取安装 gum）

通过 curl 执行:
  curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/scripts/tools/pip-sources/setup.sh | bash
EOF
}

# 解析命令行参数
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                SHOW_HELP=true
                shift
                ;;
            -v|--verbose)
                VERBOSE=true
                shift
                ;;
            *)
                print_error "未知选项: $1"
                show_help
                exit 1
                ;;
        esac
    done
}

# 检测网络连通性
check_network() {
    local url=$1
    local name=$2
    
    # 对于自定义源（从现有配置读取的），保持原始 URL 不变
    # 对于预定义源，如果 URL 不以 /simple/ 结尾，添加它
    local test_url="$url"
    
    # 检查是否是自定义源（包含 @ 符号，说明有认证信息，或者是非标准路径）
    # 或者检查 URL 是否已经包含明确的路径（不以 /simple/ 结尾但包含其他路径）
    if [[ "$url" =~ @ ]] || [[ "$url" =~ /[^/]+/[^/]+ ]] && [[ ! "$url" =~ /simple/?$ ]]; then
        # 自定义源或已有明确路径的源，保持原始 URL 不变
        test_url="$url"
    elif [[ ! "$test_url" =~ /simple/?$ ]]; then
        # 预定义源且没有明确路径，添加 /simple/
        test_url="${url%/}/simple/"
    fi
    
    if [ "$VERBOSE" = "true" ]; then
        print_debug "测试 URL: $test_url"
    fi
    
    # 使用 curl 检测网络连通性，设置超时时间
    if command -v curl &> /dev/null; then
        # 检查是否是带认证信息的自定义源
        local is_authenticated=false
        if [[ "$url" =~ @ ]]; then
            is_authenticated=true
        fi
        
        # 尝试多种方式检测：
        # 1. 先尝试 GET 请求到 /simple/ 路径（更可靠）
        # 2. 允许重定向（-L）
        # 3. 只要能够连接并返回内容即可（不要求必须是 200）
        local http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$test_url" 2>/dev/null)
        
        if [ "$VERBOSE" = "true" ]; then
            print_debug "HTTP 状态码: $http_code"
        fi
        
        # HTTP 状态码 200-399 都认为是可用的（包括重定向）
        if [ -n "$http_code" ] && [ "$http_code" -ge 200 ] && [ "$http_code" -lt 400 ]; then
            if [ "$VERBOSE" = "true" ]; then
                print_debug "$name 检测成功，HTTP 状态码: $http_code"
            fi
            return 0
        fi
        
        # 对于带认证信息的源，即使返回 400-499，只要能建立连接也认为可用
        # 因为某些源可能对直接访问有限制，但 pip 使用时是正常的
        if [ "$is_authenticated" = "true" ] && [ -n "$http_code" ] && [ "$http_code" -ge 400 ] && [ "$http_code" -lt 500 ]; then
            # 检查是否能真正建立连接（通过检查响应头）
            local response_headers=$(curl -s -I --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" "$test_url" 2>/dev/null | head -1)
            if [ -n "$response_headers" ] && [[ "$response_headers" =~ HTTP ]]; then
                if [ "$VERBOSE" = "true" ]; then
                    print_debug "$name 检测成功（带认证源，HTTP 状态码: $http_code，但能建立连接）"
                fi
                return 0
            fi
        fi
        
        # 如果上面的方法失败，尝试更宽松的方式：只要能够连接就行
        if curl -s --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" "$test_url" &> /dev/null; then
            if [ "$VERBOSE" = "true" ]; then
                print_debug "$name 检测成功（通过连接测试）"
            fi
            return 0
        fi
        
        if [ "$VERBOSE" = "true" ]; then
            print_debug "$name 检测失败"
        fi
    elif command -v wget &> /dev/null; then
        # 使用 wget 检测
        if wget --spider --timeout="$TEST_TIMEOUT" --tries=1 --quiet "$test_url" &> /dev/null; then
            if [ "$VERBOSE" = "true" ]; then
                print_debug "$name 检测成功（使用 wget）"
            fi
            return 0
        fi
    else
        # 如果没有 curl 或 wget，尝试使用 ping 检测基本网络连通性
        print_warn "未找到 curl 或 wget，跳过网络检测"
        return 0
    fi
    
    return 1
}

# 测试包的响应延迟（HTTP请求延迟）
test_package_latency() {
    local source_url=$1
    local package_name=$2
    
    if ! command -v curl &> /dev/null; then
        echo "999999"
        return 1
    fi
    
    local package_url
    package_url="$(build_package_index_url "$source_url" "$package_name")"

    # 方法1: 尝试获取包的索引页面（HTML页面，通常较小）
    local start_time=$(date +%s%N)
    local http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$package_url" 2>/dev/null)
    local end_time=$(date +%s%N)
    
    # 如果索引页面可用（200-399），计算延迟
    if [ -n "$http_code" ] && [ "$http_code" -ge 200 ] && [ "$http_code" -lt 400 ]; then
        local duration=$(( (end_time - start_time) / 1000000 ))  # 转换为毫秒
        echo "$duration"
        return 0
    fi
    
    # 方法2: 如果索引页面不可用，尝试 JSON API（某些源支持）
    local json_url="${package_url%/}/json"
    start_time=$(date +%s%N)
    http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$json_url" 2>/dev/null)
    end_time=$(date +%s%N)
    
    if [ -n "$http_code" ] && [ "$http_code" -ge 200 ] && [ "$http_code" -lt 400 ]; then
        local duration=$(( (end_time - start_time) / 1000000 ))  # 转换为毫秒
        echo "$duration"
        return 0
    fi
    
    # 方法3: 对于非标准源，尝试直接访问源根目录测试连通性
    if [[ "$source_url" =~ /pypi/ ]]; then
        start_time=$(date +%s%N)
        http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "${source_url%/}/" 2>/dev/null)
        end_time=$(date +%s%N)
        
        if [ -n "$http_code" ] && [ "$http_code" -ge 200 ] && [ "$http_code" -lt 400 ]; then
            local duration=$(( (end_time - start_time) / 1000000 ))  # 转换为毫秒
            echo "$duration"
            return 0
        fi
    fi
    
    # 如果都失败，返回一个很大的数字表示不可用
    echo "999999"
    return 1
}

# 测试包的下载速度（通过下载一个小文件来测试，返回 MB/s）
test_package_download_speed() {
    local source_url=$1
    local package_name=$2
    
    if ! command -v curl &> /dev/null; then
        echo "0"
        return 1
    fi
    
    local package_url
    package_url="$(build_package_index_url "$source_url" "$package_name")"

    # 尝试从包的索引页面中找到一个小的wheel文件来下载
    # 先获取包的索引页面
    local index_content=$(curl -s --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$package_url" 2>/dev/null)
    if [ -z "$index_content" ]; then
        echo "0"
        return 1
    fi
    
    # 尝试找到一个小的wheel文件（.whl），优先选择较小的文件
    local wheel_url=""
    local smallest_size=999999999
    local wheel_href_re='href="([^"]+\.whl)"'
    
    # 使用正则表达式查找wheel文件链接
    while IFS= read -r line; do
        if [[ "$line" =~ $wheel_href_re ]]; then
            local wheel_file="${BASH_REMATCH[1]}"
            # 构建完整的URL
            if [[ "$wheel_file" =~ ^https?:// ]]; then
                wheel_url="$wheel_file"
            else
                wheel_url="${package_url}${wheel_file}"
            fi
            
            # 尝试获取文件大小（HEAD请求）
            local file_size=$(curl -s -I --connect-timeout "$CONNECT_TIMEOUT" --max-time 5 -L "$wheel_url" 2>/dev/null | grep -i "content-length" | awk '{print $2}' | tr -d '\r\n')
            if [ -n "$file_size" ] && [ "$file_size" -gt 0 ] && [ "$file_size" -lt 5000000 ] && [ "$file_size" -lt "$smallest_size" ]; then
                smallest_size=$file_size
                # 如果找到小于1MB的文件，就使用它
                if [ "$file_size" -lt 1048576 ]; then
                    break
                fi
            fi
        fi
    done <<< "$index_content"
    
    # 如果没有找到wheel文件，尝试下载索引页面本身来测试速度
    if [ -z "$wheel_url" ] || [ "$smallest_size" -eq 999999999 ]; then
        # 下载索引页面测试速度
        local start_time=$(date +%s%N)
        local downloaded_bytes=$(curl -s --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$package_url" 2>/dev/null | wc -c)
        local end_time=$(date +%s%N)
        
        if [ "$downloaded_bytes" -gt 0 ]; then
            local duration=$(( (end_time - start_time) / 1000000 ))  # 毫秒
            if [ "$duration" -gt 0 ]; then
                # 计算速度：字节/毫秒 -> MB/s (使用 awk 避免依赖 bc)
                local speed_mbps=$(awk "BEGIN {printf \"%.2f\", $downloaded_bytes * 1000 / $duration / 1048576}" 2>/dev/null)
                if [ -n "$speed_mbps" ] && [ "$(echo "$speed_mbps" | awk '{if($1>0) print 1; else print 0}')" = "1" ]; then
                    echo "$speed_mbps"
                    return 0
                fi
            fi
        fi
        echo "0"
        return 1
    fi
    
    # 下载wheel文件测试速度
    local start_time=$(date +%s%N)
    local downloaded_bytes=$(curl -s --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$wheel_url" 2>/dev/null | wc -c)
    local end_time=$(date +%s%N)
    
    if [ "$downloaded_bytes" -gt 0 ]; then
        local duration=$(( (end_time - start_time) / 1000000 ))  # 毫秒
        if [ "$duration" -gt 0 ]; then
            # 计算速度：字节/毫秒 -> MB/s (使用 awk 避免依赖 bc)
            local speed_mbps=$(awk "BEGIN {printf \"%.2f\", $downloaded_bytes * 1000 / $duration / 1048576}" 2>/dev/null)
            if [ -n "$speed_mbps" ] && [ "$(echo "$speed_mbps" | awk '{if($1>0) print 1; else print 0}')" = "1" ]; then
                echo "$speed_mbps"
                return 0
            fi
        fi
    fi
    
    echo "0"
    return 1
}

# 探测单个源；向 stdout 输出两行：PRIMARY|…（可无正文）与 ALL|…（供并行合并）
__nlt_pip_probe_one_source() {
    local source_name=$1
    local source_url display_name latency download_speed test_url start_time end_time http_code is_authenticated response_headers
    local primary_line=""
    local all_line=""

    source_url=$(get_pip_source_url "$source_name")
    display_name=$(get_source_display_name "$source_name")

    if check_network "$source_url" "$display_name"; then
        # 避免 test_package_latency 返回非零时在 set -e 下中断（含后台并行）
        latency=$(test_package_latency "$source_url" "$TEST_PACKAGE" 2>/dev/null) || true
        download_speed="0"
        if [[ "${PIP_SOURCES_SPEED_TEST}" == "1" ]]; then
            download_speed=$(test_package_download_speed "$source_url" "$TEST_PACKAGE" 2>/dev/null || echo "0")
        fi

        if [ "$latency" = "999999" ] || [ "$latency" -ge 999999 ] 2>/dev/null; then
            test_url="$source_url"
            if [[ "$source_url" =~ @ ]] || [[ "$source_url" =~ /[^/]+/[^/]+ ]] && [[ ! "$source_url" =~ /simple/?$ ]]; then
                test_url="${source_url%/}/"
            elif [[ ! "$test_url" =~ /simple/?$ ]]; then
                if [[ "$test_url" =~ /pypi/ ]]; then
                    test_url="${test_url%/}/"
                else
                    test_url="${test_url%/}/simple/"
                fi
            fi

            start_time=$(date +%s%N)
            http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" -L "$test_url" 2>/dev/null)
            end_time=$(date +%s%N)

            is_authenticated=false
            if [[ "$source_url" =~ @ ]]; then
                is_authenticated=true
            fi

            if [ "$is_authenticated" = "true" ] && [ -n "$http_code" ] && [ "$http_code" -ge 400 ] && [ "$http_code" -lt 500 ]; then
                response_headers=$(curl -s -I --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" "$test_url" 2>/dev/null | head -1)
                if [ -n "$response_headers" ] && [[ "$response_headers" =~ HTTP ]]; then
                    latency=$(( (end_time - start_time) / 1000000 ))
                    primary_line="${source_name}|${latency}|${download_speed}|0"
                    all_line="${source_name}|${latency}|${download_speed}|可用|补充"
                else
                    all_line="${source_name}|N/A|0.00|不可用|-"
                fi
            elif [ -n "$http_code" ] && [ "$http_code" -ge 200 ] && [ "$http_code" -lt 400 ]; then
                latency=$(( (end_time - start_time) / 1000000 ))
                primary_line="${source_name}|${latency}|${download_speed}|0"
                all_line="${source_name}|${latency}|${download_speed}|可用|补充"
            elif [ -n "$http_code" ] && [ "$http_code" -ge 400 ] && [ "$http_code" -lt 500 ]; then
                response_headers=$(curl -s -I --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" "$test_url" 2>/dev/null | head -1)
                if [ -n "$response_headers" ] && [[ "$response_headers" =~ HTTP ]]; then
                    latency=$(( (end_time - start_time) / 1000000 ))
                    primary_line="${source_name}|${latency}|${download_speed}|0"
                    all_line="${source_name}|${latency}|${download_speed}|可用|补充"
                else
                    all_line="${source_name}|N/A|0.00|不可用|-"
                fi
            else
                all_line="${source_name}|N/A|0.00|不可用|-"
            fi
        else
            primary_line="${source_name}|${latency}|${download_speed}|1"
            all_line="${source_name}|${latency}|${download_speed}|可用|主源"
        fi
    else
        all_line="${source_name}|N/A|0.00|不可用|-"
    fi

    printf 'PRIMARY|%s\n' "$primary_line"
    printf 'ALL|%s\n' "$all_line"
    return 0
}


# 测试所有源的连通性、下载速度并排序
test_all_sources() {
    if [[ "${PIP_SOURCES_SPEED_TEST}" == "1" ]]; then
        print_info "正在测试所有 pip 源（连通性 + 延迟 + wheel 下载速度，顺序执行）…"
    elif [[ "${PIP_SOURCES_PARALLEL_JOBS:-8}" -le 1 ]]; then
        print_info "正在测试所有 pip 源（连通性 + 包索引延迟；顺序执行）…"
    else
        print_info "正在测试所有 pip 源（连通性 + 包索引延迟；最多 ${PIP_SOURCES_PARALLEL_JOBS} 路并行）…"
    fi

    local source_info=()
    local primary_sources=()
    local all_test_results=()

    local parallel_jobs="${PIP_SOURCES_PARALLEL_JOBS:-8}"
    if [[ "${PIP_SOURCES_SPEED_TEST}" == "1" ]] || [[ "$parallel_jobs" -le 1 ]]; then
        parallel_jobs=1
    fi

    if [[ "$parallel_jobs" -le 1 ]]; then
        local _pair_tmp pl al
        _pair_tmp="$(mktemp "${TMPDIR:-/tmp}/nlt-pip-pair.XXXXXX")"
        for source_name in "${PIP_SOURCE_NAMES[@]}"; do
            __nlt_pip_probe_one_source "$source_name" > "$_pair_tmp"
            {
                IFS= read -r pl
                IFS= read -r al
            } < "$_pair_tmp"
            if [[ -n "${pl#PRIMARY|}" ]]; then
                primary_sources+=("${pl#PRIMARY|}")
            fi
            all_test_results+=("${al#ALL|}")
        done
        rm -f "$_pair_tmp"
    else
        local _probe_dir out_i j pl al
        _probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/nlt-pip-probe.XXXXXX")"
        out_i=0
        for source_name in "${PIP_SOURCE_NAMES[@]}"; do
            __nlt_pip_probe_one_source "$source_name" > "$_probe_dir/$out_i.out" &
            ((out_i++)) || true
            if (( out_i % parallel_jobs == 0 )); then
                wait || true
            fi
        done
        wait || true
        for ((j = 0; j < out_i; j++)); do
            [[ -f "$_probe_dir/$j.out" ]] || continue
            {
                IFS= read -r pl
                IFS= read -r al
            } < "$_probe_dir/$j.out"
            if [[ -n "${pl#PRIMARY|}" ]]; then
                primary_sources+=("${pl#PRIMARY|}")
            fi
            all_test_results+=("${al#ALL|}")
        done
        rm -rf "$_probe_dir"
    fi

    if [ ${#primary_sources[@]} -eq 0 ]; then
        print_error "所有 pip 源都不可用，请检查网络连接"
        exit 1
    fi
    
    # 对主源按下载速度排序（从大到小），下载速度为 N/A 的放到后面
    if [ ${#primary_sources[@]} -gt 0 ]; then
        local sorted_lines=()
        local temp_array=("${primary_sources[@]}")

        for i in "${!temp_array[@]}"; do
            local source latency speed has_pkg has_speed sort_key speed_check speed_num sort_value latency_num
            source=$(echo "${temp_array[$i]}" | LC_ALL=C cut -d'|' -f1)
            latency=$(echo "${temp_array[$i]}" | LC_ALL=C cut -d'|' -f2)
            speed=$(echo "${temp_array[$i]}" | LC_ALL=C cut -d'|' -f3)
            has_pkg=$(echo "${temp_array[$i]}" | LC_ALL=C cut -d'|' -f4)

            has_speed=0
            sort_key=""
            if [ -n "$speed" ] && [ "$speed" != "0" ] && [ "$speed" != "0.00" ] && [ "$speed" != "N/A" ]; then
                speed_check=$(echo "$speed" | awk '{if($1>0) print 1; else print 0}' 2>/dev/null)
                if [ "$speed_check" = "1" ]; then
                    has_speed=1
                    speed_num=$(echo "$speed" | awk '{print $1}' 2>/dev/null || echo "0")
                    sort_value=$(awk "BEGIN {printf \"%.0f\", (1000 - $speed_num) * 1000}" 2>/dev/null || echo "1000000")
                    sort_key=$(awk "BEGIN {printf \"%020d\", $sort_value}" 2>/dev/null || echo "00000000000001000000")
                fi
            fi
            if [ "$has_speed" = "0" ]; then
                latency_num=$(echo "$latency" | awk '{print int($1+0.5)}' 2>/dev/null || echo "999999")
                sort_key=$(awk "BEGIN {printf \"%020d\", 2000000 + $latency_num}" 2>/dev/null || echo "2999999")
            fi
            sorted_lines+=("${sort_key}|${source}|${latency}|${speed}|${has_pkg}")
        done

        local _ps_sort_tmp
        _ps_sort_tmp="$(mktemp "${TMPDIR:-/tmp}/nlt-pip-sort.XXXXXX")"
        printf '%s\n' "${sorted_lines[@]}" | LC_ALL=C sort -t'|' -k1,1 >"$_ps_sort_tmp"
        primary_sources=()
        while IFS= read -r item || [[ -n "$item" ]]; do
            [[ -z "$item" ]] && continue
            source=$(echo "$item" | LC_ALL=C cut -d'|' -f2)
            latency=$(echo "$item" | LC_ALL=C cut -d'|' -f3)
            speed=$(echo "$item" | LC_ALL=C cut -d'|' -f4)
            has_pkg=$(echo "$item" | LC_ALL=C cut -d'|' -f5)
            primary_sources+=("${source}|${latency}|${speed}|${has_pkg}")
        done <"$_ps_sort_tmp"
        rm -f "$_ps_sort_tmp"
    fi
    
    # 显示检测结果表格
    echo ""
    print_info "检测结果汇总:"
    echo ""
    
    # 打印表格标题（中文名称放到最后一列）
    printf "%-6s %-18s %-12s %-12s %-12s %s\n" "序号" "源标识" "状态" "延迟" "下载速度" "源名称"
    printf "%-6s %-18s %-12s %-12s %-12s %s\n" "------" "------------------" "------------" "------------" "------------" "------------------------------"
    
    local index=1
    
    # 显示主源（按下载速度排序，N/A 在后）
    for item in "${primary_sources[@]}"; do
        local source=$(echo "$item" | LC_ALL=C cut -d'|' -f1)
        local latency=$(echo "$item" | LC_ALL=C cut -d'|' -f2)
        local speed=$(echo "$item" | LC_ALL=C cut -d'|' -f3)
        local display_name=$(get_source_display_name "$source")
        
        # 格式化下载速度显示
        local speed_display="N/A"
        if [ -n "$speed" ] && [ "$speed" != "0" ] && [ "$speed" != "0.00" ]; then
            # 使用 awk 检查速度是否大于 0
            if [ "$(echo "$speed" | awk '{if($1>0) print 1; else print 0}')" = "1" ]; then
                speed_display="${speed}MB/s"
            fi
        fi
        
        printf "%-6s %-18s %-12s %-12s %-12s %s\n" "$index" "$source" "✓ 可用" "${latency}ms" "$speed_display" "$display_name"
        ((index++))
    done
    
    # 不再显示补充源（网络不可用或延迟N/A的源不加入配置）
    
    # 显示不可用的源（从 all_test_results 中提取）
    # all_test_results 格式：源名|延迟|下载速度|状态|类型
    for result in "${all_test_results[@]}"; do
        local source=$(echo "$result" | LC_ALL=C cut -d'|' -f1)
        local status=$(echo "$result" | LC_ALL=C cut -d'|' -f4)  # 状态在第4个字段
        
        # 检查是否已经在主源中
        local found=false
        for item in "${primary_sources[@]}"; do
            local existing_source=$(echo "$item" | LC_ALL=C cut -d'|' -f1)
            if [ "$existing_source" = "$source" ]; then
                found=true
                break
            fi
        done
        
        if [ "$status" = "不可用" ] && [ "$found" = "false" ]; then
            local display_name=$(get_source_display_name "$source")
            
            printf "%-6s %-18s %-12s %-12s %-12s %s\n" "$index" "$source" "✗ 不可用" "N/A" "N/A" "$display_name"
            ((index++))
        fi
    done
    
    echo ""
    
    if [ ${#primary_sources[@]} -gt 0 ]; then
        local fastest_source=$(echo "${primary_sources[0]}" | LC_ALL=C cut -d'|' -f1)
        local fastest_latency=$(echo "${primary_sources[0]}" | LC_ALL=C cut -d'|' -f2)
        local fastest_speed=$(echo "${primary_sources[0]}" | LC_ALL=C cut -d'|' -f3)
        local speed_info=""
        if [ -n "$fastest_speed" ] && [ "$fastest_speed" != "0" ] && [ "$fastest_speed" != "0.00" ]; then
            if [ "$(echo "$fastest_speed" | awk '{if($1>0) print 1; else print 0}')" = "1" ]; then
                speed_info="，下载速度: ${fastest_speed}MB/s"
            fi
        fi
        print_info "最快的源: $fastest_source (延迟: ${fastest_latency}ms${speed_info})"
        echo ""
    fi
    
    # 返回排序后的源列表（通过全局变量）
    # 格式：主源（按下载速度排序，N/A 在后），网络不可用或延迟N/A的源保存到 UNAVAILABLE_SOURCES
    AVAILABLE_SOURCES=()
    SOURCE_SPEEDS=()
    SOURCE_HAS_PACKAGE=()
    UNAVAILABLE_SOURCES=()  # 不可用的源列表（保存到配置文件的注释中）
    
    # 添加主源（包括能响应但无法下载包的源）
    for item in "${primary_sources[@]}"; do
        local source=$(echo "$item" | LC_ALL=C cut -d'|' -f1)
        local latency=$(echo "$item" | LC_ALL=C cut -d'|' -f2)
        local has_pkg=$(echo "$item" | LC_ALL=C cut -d'|' -f4)

        # 明文 HTTP 源默认排除。测速时明文往往更快，很容易夺得第一名而被写成
        # index-url，于是此后每一次 pip install 都在明文信道上取 wheel，且
        # trusted-host 还压掉了 pip 的 TLS 告警 —— 同网段攻击者即可注入恶意
        # wheel 并取得用户权限的代码执行。这个配置是持久写进 pip.conf 的。
        if _pip_source_is_insecure "$source"; then
            if [ "${PIP_SOURCES_ALLOW_INSECURE:-}" = "1" ]; then
                print_warn "已启用明文 HTTP 源 $source（PIP_SOURCES_ALLOW_INSECURE=1）：该源的下载可被中间人篡改。"
            else
                print_warn "跳过明文 HTTP 源 $source（如确需使用请设置 PIP_SOURCES_ALLOW_INSECURE=1）"
                UNAVAILABLE_SOURCES+=("$source")
                continue
            fi
        fi

        AVAILABLE_SOURCES+=("$source")
        SOURCE_SPEEDS+=("$latency")
        SOURCE_HAS_PACKAGE+=("$has_pkg")
    done
    
    # 收集不可用的源（保存到配置文件的注释中，避免丢失）
    for result in "${all_test_results[@]}"; do
        local source=$(echo "$result" | LC_ALL=C cut -d'|' -f1)
        local status=$(echo "$result" | LC_ALL=C cut -d'|' -f4)  # 状态在第4个字段
        
        # 检查是否已经在可用源中
        local found=false
        for available_source in "${AVAILABLE_SOURCES[@]}"; do
            if [ "$available_source" = "$source" ]; then
                found=true
                break
            fi
        done
        
        # 如果是不可用的源且不在可用源列表中，添加到不可用源列表
        if [ "$status" = "不可用" ] && [ "$found" = "false" ]; then
            UNAVAILABLE_SOURCES+=("$source")
        fi
    done
    
}

# 从现有配置文件中读取源
read_existing_sources() {
    local config_file="$1"
    local existing_sources=()
    local unavailable_source_re='^[[:space:]]*#[[:space:]]*unavailable-source:[[:space:]]*(https?://[^[:space:]]+)'
    local extra_index_re='^[[:space:]]*extra-index-url[[:space:]]*=[[:space:]]*(.+)'
    
    if [ ! -f "$config_file" ]; then
        EXISTING_SOURCES=()
        return 0
    fi
    
    # 读取 index-url
    local index_url=$(grep -E "^[[:space:]]*index-url[[:space:]]*=" "$config_file" 2>/dev/null | sed 's/^[^=]*=[[:space:]]*//' | sed 's/[[:space:]]*$//' | head -1)
    if [ -n "$index_url" ]; then
        existing_sources+=("$index_url")
    fi
    
    # 读取 extra-index-url（可能有多行）
    local in_extra_index=false
    while IFS= read -r line || [ -n "$line" ]; do
        # 跳过空行
        if [[ -z "${line// }" ]]; then
            continue
        fi
        
        # 读取注释中的不可用源（格式：# unavailable-source: URL  # 说明）
        if [[ "$line" =~ $unavailable_source_re ]]; then
            local url="${BASH_REMATCH[1]}"
            if [ -n "$url" ]; then
                existing_sources+=("$url")
            fi
            continue
        fi
        
        # 跳过普通注释
        if [[ "$line" =~ ^[[:space:]]*# ]]; then
            continue
        fi
        
        # 匹配 extra-index-url 行
        if [[ "$line" =~ $extra_index_re ]]; then
            in_extra_index=true
            local urls="${BASH_REMATCH[1]}"
            # 分割多个 URL（可能在同一行，用空格分隔）
            for url in $urls; do
                url=$(echo "$url" | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
                if [ -n "$url" ] && [[ "$url" =~ ^https?:// ]]; then
                    existing_sources+=("$url")
                fi
            done
        elif [[ "$in_extra_index" = "true" ]] && [[ "$line" =~ ^[[:space:]]+https?:// ]]; then
            # 续行（extra-index-url 的续行）
            local url=$(echo "$line" | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
            if [ -n "$url" ] && [[ "$url" =~ ^https?:// ]]; then
                existing_sources+=("$url")
            fi
        elif [[ "$line" =~ ^[[:space:]]*\[ ]]; then
            # 新的配置段开始，结束 extra-index-url
            in_extra_index=false
        fi
    done < "$config_file"
    
    # 返回找到的源（通过全局变量）
    EXISTING_SOURCES=("${existing_sources[@]}")
}

# 为自定义源生成标识符和显示名称
generate_custom_source_id() {
    local url=$1
    # 使用 URL 的哈希值作为标识符（前8位）
    if command -v md5sum &> /dev/null; then
        local hash=$(echo -n "$url" | md5sum 2>/dev/null | cut -c1-8)
    elif command -v md5 &> /dev/null; then
        local hash=$(echo -n "$url" | md5 2>/dev/null | cut -c1-8)
    elif command -v shasum &> /dev/null; then
        local hash=$(echo -n "$url" | shasum -a 256 2>/dev/null | cut -c1-8)
    else
        # 如果没有哈希工具，使用 URL 的简化版本
        local hash=$(echo -n "$url" | tr -cd 'a-zA-Z0-9' | cut -c1-8)
    fi
    echo "custom-${hash}"
}

# 获取自定义源的显示名称（隐藏密码）
get_custom_source_display_name() {
    local url=$1
    local auth_url_re='^https?://([^:]+):([^@]+)@(.+)$'
    local host_url_re='^https?://([^/]+)'
    # 如果 URL 包含 @，说明有认证信息，隐藏密码部分
    if [[ "$url" =~ $auth_url_re ]]; then
        local user="${BASH_REMATCH[1]}"
        local host="${BASH_REMATCH[3]}"
        echo "自定义源 (${user}@${host})"
    elif [[ "$url" =~ $host_url_re ]]; then
        local host="${BASH_REMATCH[1]}"
        echo "自定义源 (${host})"
    else
        echo "自定义源"
    fi
}

# 添加自定义源到检测列表（与预定义 URL 相同的不重复；自定义项一次性插到列表前）
add_custom_sources_to_list() {
    if [ ${#EXISTING_SOURCES[@]} -eq 0 ]; then
        return 0
    fi

    print_info "从现有配置中读取到 ${#EXISTING_SOURCES[@]} 个 URL，正在合并到检测列表…"

    local custom_entries=()
    local custom_ids=()
    local url found predefined_name predefined_url source_id display_name

    for url in "${EXISTING_SOURCES[@]}"; do
        [[ -z "$url" ]] && continue
        found=false
        for predefined_name in "${PIP_PREDEFINED_SOURCES[@]}"; do
            predefined_url=$(get_pip_source_url "$predefined_name" 2>/dev/null) || true
            if [[ "$url" == "$predefined_url" ]]; then
                found=true
                break
            fi
        done
        if [[ "$found" == true ]]; then
            continue
        fi
        source_id=$(generate_custom_source_id "$url")
        display_name=$(get_custom_source_display_name "$url")
        custom_entries+=("${source_id}|${url}|${display_name}")
        custom_ids+=("$source_id")
    done

    CUSTOM_SOURCES=("${custom_entries[@]}")
    if [ ${#custom_ids[@]} -gt 0 ]; then
        PIP_SOURCE_NAMES=("${custom_ids[@]}" "${PIP_PREDEFINED_SOURCES[@]}")
        print_info "已加入 ${#custom_ids[@]} 个自定义源（将优先测速）"
    else
        PIP_SOURCE_NAMES=("${PIP_PREDEFINED_SOURCES[@]}")
    fi
}

# 确定 pip 配置文件路径
determine_pip_config_path() {
    # pip 配置文件可能的路径（按优先级）
    local possible_paths=(
        "$HOME/.pip/pip.conf"
        "$HOME/.config/pip/pip.conf"
        "/etc/pip.conf"
    )
    
    # 优先使用用户目录下的配置
    if [ -f "$HOME/.pip/pip.conf" ]; then
        PIP_CONFIG_DIR="$HOME/.pip"
        PIP_CONFIG_FILE="$HOME/.pip/pip.conf"
    elif [ -f "$HOME/.config/pip/pip.conf" ]; then
        PIP_CONFIG_DIR="$HOME/.config/pip"
        PIP_CONFIG_FILE="$HOME/.config/pip/pip.conf"
    else
        # 如果文件不存在，创建 ~/.pip/pip.conf（这是最常见的路径）
        PIP_CONFIG_DIR="$HOME/.pip"
        PIP_CONFIG_FILE="$HOME/.pip/pip.conf"
    fi
    
    print_info "pip 配置文件路径: $PIP_CONFIG_FILE"
}


# 备份现有配置
backup_config() {
    if [ -f "$PIP_CONFIG_FILE" ]; then
        local backup_file="${PIP_CONFIG_FILE}${BACKUP_SUFFIX}"
        print_info "备份现有配置到: $backup_file"
        cp "$PIP_CONFIG_FILE" "$backup_file"
        if [ $? -eq 0 ]; then
            print_info "备份成功"
            
            # 清理旧备份，只保留最近3份
            local backup_base=$(basename "$PIP_CONFIG_FILE")
            local backup_files=()
            
            # 使用 ls -t 按修改时间排序（最新的在前），兼容 macOS 和 Linux
            # 写入临时文件而非 < <(...)，兼容「sh 实为 bash --posix」时禁用进程替换的情况
            local _ls_tmp
            _ls_tmp="$(mktemp "${TMPDIR:-/tmp}/nlt-pip-ls.XXXXXX")"
            ls -t "$PIP_CONFIG_DIR"/"${backup_base}".backup.* 2>/dev/null >"$_ls_tmp" || true
            while IFS= read -r file; do
                [ -n "$file" ] && [ -f "$file" ] && backup_files+=("$file")
            done <"$_ls_tmp"
            rm -f "$_ls_tmp"
            
            # 如果备份文件超过3个，删除最旧的
            local backup_count=${#backup_files[@]}
            if [ $backup_count -gt 3 ]; then
                local to_delete=$((backup_count - 3))
                print_info "发现 $backup_count 个备份文件，保留最近3份，删除 $to_delete 个旧备份..."
                for ((i=3; i<backup_count; i++)); do
                    if [ -f "${backup_files[$i]}" ]; then
                        rm -f "${backup_files[$i]}"
                        if [ $? -eq 0 ]; then
                            print_info "已删除旧备份: $(basename "${backup_files[$i]}")"
                        fi
                    fi
                done
            fi
        else
            print_error "备份失败"
            exit 1
        fi
    fi
}

# 准备多源配置（不再需要选择，使用所有可用源）
prepare_multi_source_config() {
    # 如果没有可用的源列表，先测试
    if [ ${#AVAILABLE_SOURCES[@]} -eq 0 ]; then
        test_all_sources
    fi
    
    if [ ${#AVAILABLE_SOURCES[@]} -eq 0 ]; then
        print_error "没有可用的 pip 源"
        exit 1
    fi
    
    print_info "将配置 ${#AVAILABLE_SOURCES[@]} 个可用源（已按测速结果排序；主源为最快项）"
}

# 创建配置目录
create_config_directory() {
    if [ ! -d "$PIP_CONFIG_DIR" ]; then
        print_info "创建配置目录: $PIP_CONFIG_DIR"
        mkdir -p "$PIP_CONFIG_DIR"
        if [ $? -ne 0 ]; then
            print_error "创建配置目录失败"
            exit 1
        fi
    fi
}


# 生成 pip 配置文件内容
generate_pip_config_content() {
    local config_content=""
    local auth_host_re='https?://([^@]+)@([^/]+)'
    local host_re='https?://([^/]+)'
    
    # 文件头注释
    config_content+="# pip 配置文件\n"
    config_content+="# 由 nlt-pip-sources / pip-sources/setup.sh 自动生成\n"
    config_content+="# 生成时间: $(date '+%Y-%m-%d %H:%M:%S')\n"
    config_content+="# 配置了 ${#AVAILABLE_SOURCES[@]} 个可用源（按测速结果排序）\n"
    if [ ${#UNAVAILABLE_SOURCES[@]} -gt 0 ]; then
        config_content+="# 另有 ${#UNAVAILABLE_SOURCES[@]} 个不可用源已保存到注释中（避免丢失）\n"
    fi
    config_content+="\n[global]\n"
    
    # 第一个源作为主源（index-url）
    if [ ${#AVAILABLE_SOURCES[@]} -gt 0 ]; then
        local first_source="${AVAILABLE_SOURCES[0]}"
        local first_url=$(get_pip_source_url "$first_source")
        config_content+="index-url = $first_url\n"
    fi
    
    # 其他源作为额外源（extra-index-url）
    if [ ${#AVAILABLE_SOURCES[@]} -gt 1 ]; then
        config_content+="extra-index-url ="
        for i in $(seq 1 $((${#AVAILABLE_SOURCES[@]} - 1))); do
            local source_name="${AVAILABLE_SOURCES[$i]}"
            local source_url=$(get_pip_source_url "$source_name")
            if [ $i -eq 1 ]; then
                config_content+=" $source_url\n"
            else
                config_content+="                $source_url\n"
            fi
        done
    fi
    
    # trusted-host —— 只为明文 HTTP 源生成。
    # 原实现把**所有**源的主机名都列进 trusted-host，包括 HTTPS 源。对 HTTPS
    # 主机而言 trusted-host 的作用是让 pip 跳过该主机的证书校验，纯属自伤：
    # 它把本来安全的连接降级为可被中间人篡改。pip 只有在 index 走 http://
    # （或证书确实无法校验）时才需要这一项。
    local first_host=true
    local seen_hosts=()
    local trusted_block=""
    for source_name in "${AVAILABLE_SOURCES[@]}"; do
        _pip_source_is_insecure "$source_name" || continue
        local source_url=$(get_pip_source_url "$source_name")
        # 提取主机名，处理带认证信息的 URL（如 https://user:pass@host.com）
        local host=""
        if [[ "$source_url" =~ $auth_host_re ]]; then
            # 带认证信息的 URL
            host="${BASH_REMATCH[2]}"
        elif [[ "$source_url" =~ $host_re ]]; then
            # 普通 URL
            host="${BASH_REMATCH[1]}"
        fi

        # 去重（同一个主机只添加一次）
        if [ -n "$host" ]; then
            local host_seen=false
            for seen_host in "${seen_hosts[@]}"; do
                if [ "$seen_host" = "$host" ]; then
                    host_seen=true
                    break
                fi
            done

            if [ "$host_seen" = "false" ]; then
                seen_hosts+=("$host")
                if [ "$first_host" = "true" ]; then
                    trusted_block+=" $host\n"
                    first_host=false
                else
                    trusted_block+="              $host\n"
                fi
            fi
        fi
    done

    # 没有明文源时完全不写 trusted-host —— 空的 `trusted-host =` 既无意义，
    # 也会让人误以为这些主机被豁免了证书校验。
    if [ -n "$trusted_block" ]; then
        config_content+="# 以下主机通过明文 HTTP 访问，需要 trusted-host；其余 HTTPS 源不应列入。\n"
        config_content+="trusted-host =${trusted_block}"
    fi
    config_content+="\n"
    
    # 保存不可用的源到注释中（避免因检测问题丢失）
    # 格式：# unavailable-source: URL  # 说明
    if [ ${#UNAVAILABLE_SOURCES[@]} -gt 0 ]; then
        config_content+="# 以下源在检测时不可用，但已保存以避免丢失（可能是临时网络问题）\n"
        config_content+="# 如果这些源恢复可用，下次运行脚本时会自动检测并启用\n"
        for source_name in "${UNAVAILABLE_SOURCES[@]}"; do
            local source_url=$(get_pip_source_url "$source_name")
            local display_name=$(get_source_display_name "$source_name")
            config_content+="# unavailable-source: $source_url  # $display_name ($source_name)\n"
        done
        config_content+="\n"
    fi
    
    # 输出配置内容
    printf "%b" "$config_content"
}


# 写入 pip 配置（多源配置）
write_pip_config() {
    print_info "正在写入多源配置..."
    
    # 确保配置目录存在
    if [ ! -d "$PIP_CONFIG_DIR" ]; then
        print_info "创建配置目录: $PIP_CONFIG_DIR"
        if ! mkdir -p "$PIP_CONFIG_DIR" 2>&1; then
            print_error "创建配置目录失败: $PIP_CONFIG_DIR"
            print_error "请检查目录权限或手动创建目录: mkdir -p $PIP_CONFIG_DIR"
            exit 1
        fi
    fi
    
    # 检查目录是否可写
    if [ ! -w "$PIP_CONFIG_DIR" ]; then
        print_error "配置目录不可写: $PIP_CONFIG_DIR"
        print_error "请检查目录权限: ls -ld $PIP_CONFIG_DIR"
        exit 1
    fi
    
    # 生成配置内容
    local config_content=""
    config_content=$(generate_pip_config_content)
    
    # 创建临时文件，使用更安全的路径
    local temp_file="${PIP_CONFIG_DIR}/pip.conf.tmp.$$"
    
    # 使用 printf 写入文件，更可靠
    if ! printf "%b" "$config_content" > "$temp_file" 2>&1; then
        print_error "pip 配置写入失败（无法写入临时文件: $temp_file）"
        print_error "错误详情: $(cat "$temp_file" 2>&1 | head -5)"
        [ -f "$temp_file" ] && rm -f "$temp_file" 2>/dev/null
        exit 1
    fi
    
    # 检查临时文件是否存在
    if [ ! -f "$temp_file" ]; then
        print_error "pip 配置写入失败（临时文件不存在: $temp_file）"
        print_error "可能的原因：目录权限不足或磁盘空间不足"
        exit 1
    fi
    
    # 检查临时文件是否有内容
    if [ ! -s "$temp_file" ]; then
        print_error "pip 配置写入失败（临时文件为空）"
        rm -f "$temp_file" 2>/dev/null
        exit 1
    fi
    
    # 将临时文件移动到目标位置（原子操作）
    local mv_output=""
    if ! mv_output=$(mv "$temp_file" "$PIP_CONFIG_FILE" 2>&1); then
        print_error "pip 配置写入失败（无法移动到目标位置: $PIP_CONFIG_FILE）"
        if [ -n "$mv_output" ]; then
            print_error "移动错误: $mv_output"
        fi
        print_error "可能的原因：目标目录权限不足或目标文件被锁定"
        [ -f "$temp_file" ] && rm -f "$temp_file" 2>/dev/null
        exit 1
    fi
    
    # 验证最终文件是否存在
    if [ ! -f "$PIP_CONFIG_FILE" ]; then
        print_error "pip 配置写入失败（目标文件不存在: $PIP_CONFIG_FILE）"
        exit 1
    fi
    
    # 简单校验：检查文件是否可读且非空
    if [ ! -r "$PIP_CONFIG_FILE" ]; then
        print_error "pip 配置写入失败（目标文件不可读: $PIP_CONFIG_FILE）"
        exit 1
    fi
    
    if [ ! -s "$PIP_CONFIG_FILE" ]; then
        print_error "pip 配置写入失败（目标文件为空）"
        exit 1
    fi
    
    print_info "pip 多源配置写入成功"
}

# 验证配置
verify_config() {
    print_info "验证配置..."
    
    if [ -f "$PIP_CONFIG_FILE" ]; then
        print_info "配置文件内容:"
        echo ""
        cat "$PIP_CONFIG_FILE"
        echo ""
        
        # 尝试读取配置（如果 pip 可用）
        if command -v pip &> /dev/null; then
            print_info "使用 pip config list 验证配置:"
            pip config list 2>/dev/null || print_warn "无法读取 pip 配置（可能 pip 未安装）"
        else
            print_warn "未检测到 pip 命令，无法验证配置"
        fi
    else
        print_error "配置文件不存在"
        exit 1
    fi
}

# 显示配置信息
show_info() {
    print_info "pip 多源配置完成！"
    echo ""
    echo "配置文件路径: $PIP_CONFIG_FILE"
    echo "已配置 ${#AVAILABLE_SOURCES[@]} 个源:"
    local index=1
    for i in "${!AVAILABLE_SOURCES[@]}"; do
        local source_name="${AVAILABLE_SOURCES[$i]}"
        local speed="${SOURCE_SPEEDS[$i]}"
        local has_package="${SOURCE_HAS_PACKAGE[$i]}"
        local source_url=$(get_pip_source_url "$source_name")
        local display_name=$(get_source_display_name "$source_name")
        
        if [ "$has_package" = "1" ]; then
            echo "  $index. $display_name ($source_name) - ${speed}ms [主源]"
        else
            echo "  $index. $display_name ($source_name) [补充源]"
        fi
        ((index++))
    done
    if [ -f "${PIP_CONFIG_FILE}${BACKUP_SUFFIX}" ]; then
        echo ""
        echo "备份文件: ${PIP_CONFIG_FILE}${BACKUP_SUFFIX}"
    fi
    echo ""
    print_info "要使用此配置，请运行: pip install <package_name>"
    echo ""
    print_info "pip 会按顺序尝试这些源，优先使用最快的源"
    echo ""
}

# 使用最近一次备份恢复 pip 配置（工具规范 uninstall）
cmd_pip_uninstall_restore() {
    parse_args "$@"
    if [ "$SHOW_HELP" = "true" ]; then
        show_help
        exit 0
    fi
    determine_pip_config_path
    local base_name backup_file
    base_name=$(basename "$PIP_CONFIG_FILE")
    backup_file=$(ls -t "${PIP_CONFIG_DIR}/${base_name}".backup.* 2>/dev/null | head -1 || true)
    if [ -z "$backup_file" ] || [ ! -f "$backup_file" ]; then
        print_error "未找到 ${PIP_CONFIG_DIR}/${base_name}.backup.*，无法自动恢复。"
        exit 1
    fi
    if [ "${NONINTERACTIVE:-}" != "1" ]; then
        nlt_ui_confirm "用备份恢复 pip 配置？ ${backup_file} -> ${PIP_CONFIG_FILE}" || return 0
    fi
    cp "$backup_file" "$PIP_CONFIG_FILE"
    print_info "已从备份恢复: ${PIP_CONFIG_FILE}"
}

nlt_cli_main() {
    if [ $# -eq 0 ]; then
        _nlt_ensure_gum || exit 1
        local pick
        pick=$(gum choose --header "pip 源配置（nltdeploy 工具）" \
            "install" "update" "reinstall" "uninstall" "help") || exit 0
        [ -z "$pick" ] && exit 0
        set -- "$pick"
    fi
    case "$1" in
        help | -h | --help)
            show_help
            exit 0
            ;;
        install | update | upgrade | reinstall)
            shift
            main "$@"
            ;;
        uninstall)
            shift
            cmd_pip_uninstall_restore "$@"
            ;;
        *)
            main "$@"
            ;;
    esac
}

# 主函数
main() {
    # 解析命令行参数
    parse_args "$@"
    
    # 如果显示帮助，直接退出
    if [ "$SHOW_HELP" = "true" ]; then
        show_help
        exit 0
    fi

    _nlt_ensure_gum || exit 1
    
    # 如果指定了单个源，提示用户现在支持多源配置
    if [ -n "$SELECTED_SOURCE" ]; then
        print_warn "注意：脚本现在支持多源配置，将自动检测并配置所有可用源"
        print_info "如果只想使用单个源，可以手动编辑配置文件"
        echo ""
    fi
    
    print_info "开始配置 pip 多源..."
    echo ""
    
    # 确定配置文件路径
    determine_pip_config_path
    
    # 从现有配置中读取源（避免源丢失）
    read_existing_sources "$PIP_CONFIG_FILE"
    
    # 将自定义源添加到检测列表
    add_custom_sources_to_list
    
    # 测试所有源的连通性和下载速度
    test_all_sources
    
    # 准备多源配置
    prepare_multi_source_config
    
    # 备份现有配置
    backup_config
    
    # 创建配置目录
    create_config_directory
    
    # 直接写入配置
    write_pip_config
    
    # 验证配置
    verify_config
    
    # 显示信息
    show_info
    
    print_info "脚本执行完成！"
}

# 执行主函数（无参时 gum 选子命令）
nlt_cli_main "$@"
