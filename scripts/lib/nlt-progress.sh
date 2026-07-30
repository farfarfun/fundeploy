#!/usr/bin/env bash
# nlt-progress：可复用终端进度条（macOS + Linux）。由其他脚本 source。
[[ -n "${_NLT_PROGRESS_LOADED:-}" ]] && return 0
_NLT_PROGRESS_LOADED=1

_NLT_PROGRESS_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=nlt-github-download.sh
source "${_NLT_PROGRESS_LIB_DIR}/nlt-github-download.sh"

_nlt_file_size() {
  local f="$1"
  [[ -f "$f" ]] || { echo 0; return 0; }
  if stat -f%z "$f" >/dev/null 2>&1; then
    stat -f%z "$f"
  else
    stat -c%s "$f" 2>/dev/null || echo 0
  fi
}

_nlt_pb_now_s() {
  date +%s
}

_nlt_pb_cols() {
  if [[ -n "${COLUMNS:-}" ]] && [[ "${COLUMNS}" =~ ^[0-9]+$ ]]; then
    echo "${COLUMNS}"
  elif command -v tput >/dev/null 2>&1 && [[ -t 1 ]]; then
    tput cols 2>/dev/null || echo 80
  else
    echo 80
  fi
}

# 字节 → 人类可读（1024 底，一位小数）
nlt_pb_human_bytes() {
  awk -v n="${1:-0}" 'BEGIN{
    if (n < 0) n = 0
    u[1]="KiB"; u[2]="MiB"; u[3]="GiB"; u[4]="TiB"
    if (n < 1024) { printf "%d B", n; exit }
    x = n / 1024.0
    i = 1
    while (x >= 1024 && i < 4) { x /= 1024; i++ }
    printf "%.1f %s", x, u[i]
  }'
}

_nlt_pb_fmt_hms() {
  awk -v s="${1:-0}" 'BEGIN{
    if (s < 0) s = 0
    h = int(s / 3600)
    m = int((s % 3600) / 60)
    sec = int(s % 60)
    if (h > 0) printf "%d:%02d:%02d", h, m, sec
    else printf "%d:%02d", m, sec
  }'
}

_nlt_pb_parse_content_length() {
  local url="$1"
  url="$(_nlt_github_download_resolve_url "$url")"
  curl -sI -L "$url" 2>/dev/null | awk 'BEGIN{IGNORECASE=1} /^content-length:/ {v=$2+0} END{print v+0}'
}

# nlt_pb_render current_bytes total_bytes label start_epoch
# total_bytes=0 表示总长未知：条形为不确定样式，百分比与 ETA 为 —
# 默认条形字符为单宽 ASCII（# / -），避免在 CJK/UTF-8 终端上 █/░ 被计为双宽列
# 导致 \r 重绘错位、行尾出现“漂移”块字符。需要 Unicode 块时可设 NLT_PB_USE_UNICODE=1。
nlt_pb_render() {
  local cur="${1:-0}" total="${2:-0}" label="${3:-}" start="${4:-0}"
  local now cols barw filled pct elapsed rate eta rem slot span
  local cur_h total_h rate_h line bar i color denom maxlab ch_f ch_e

  [[ "${cur}" =~ ^[0-9]+$ ]] || cur=0
  [[ "${total}" =~ ^[0-9]+$ ]] || total=0
  [[ "${start}" =~ ^[0-9]+$ ]] || start="$(_nlt_pb_now_s)"

  now="$(_nlt_pb_now_s)"
  elapsed=$((now - start))
  [[ "$elapsed" -lt 1 ]] && elapsed=1

  cur_h="$(nlt_pb_human_bytes "$cur")"

  if [[ ! -t 1 ]]; then
    return 0
  fi

  cols="$(_nlt_pb_cols)"
  # 为右侧统计、括号与标签预留列；避免整行超过终端宽度导致折行错乱
  maxlab=$((cols - 58))
  [[ "$maxlab" -lt 10 ]] && maxlab=10
  if [[ "${#label}" -gt "$maxlab" ]]; then
    label="${label:0:$((maxlab - 3))}..."
  fi

  barw=$((cols - 56))
  [[ "$barw" -lt 8 ]] && barw=8
  [[ "$barw" -gt 32 ]] && barw=32

  if [[ "${NLT_PB_USE_UNICODE:-}" == "1" ]]; then
    ch_f=$'█'
    ch_e=$'░'
  else
    ch_f='#'
    ch_e='-'
  fi

  denom=$((barw - 1))
  [[ "$denom" -lt 1 ]] && denom=1

  rate=$(awk -v c="$cur" -v e="$elapsed" 'BEGIN { if (e > 0) printf "%.0f", c / e; else print 0 }')
  rate_h="$(nlt_pb_human_bytes "$rate")/s"

  if [[ "$total" -gt 0 ]]; then
    pct=$(awk -v c="$cur" -v t="$total" 'BEGIN {
      if (t <= 0) { print 0; exit }
      p = 100.0 * c / t
      if (p > 100) p = 100
      printf "%.1f", p
    }')
    total_h="$(nlt_pb_human_bytes "$total")"
    rem=$((total - cur))
    [[ "$rem" -lt 0 ]] && rem=0
    if [[ "$rate" -gt 0 ]]; then
      eta=$(awk -v r="$rem" -v rt="$rate" 'BEGIN { if (rt > 0) printf "%.0f", r / rt; else print 0 }')
      eta="$(_nlt_pb_fmt_hms "$eta")"
    else
      eta="—"
    fi
    filled=$(awk -v c="$cur" -v t="$total" -v w="$barw" 'BEGIN {
      if (t <= 0) { print 0; exit }
      f = int(w * c / t + 0.5)
      if (f > w) f = w
      if (f < 0) f = 0
      print f
    }')
  else
    pct="—"
    total_h="?"
    eta="—"
    # 不确定：滑动的 4 格亮块
    span=4
    slot=$(( (elapsed * 2) % (barw - span + 1) ))
    filled=-1
  fi

  bar=""
  if [[ "$filled" -ge 0 ]]; then
    for ((i = 0; i < barw; i++)); do
      if [[ "$i" -lt "$filled" ]]; then
        color=$((39 + (i * 6 / denom)))
        [[ "$color" -gt 45 ]] && color=45
        bar+="$(printf '\033[38;5;%dm%s\033[0m' "$color" "$ch_f")"
      else
        bar+="$(printf '\033[38;5;238m%s\033[0m' "$ch_e")"
      fi
    done
  else
    for ((i = 0; i < barw; i++)); do
      if [[ "$i" -ge "$slot" && "$i" -lt $((slot + span)) ]]; then
        color=$((39 + (i * 6 / denom)))
        [[ "$color" -gt 45 ]] && color=45
        bar+="$(printf '\033[38;5;%dm%s\033[0m' "$color" "$ch_f")"
      else
        bar+="$(printf '\033[38;5;238m%s\033[0m' "$ch_e")"
      fi
    done
  fi

  line="$(printf '\r\033[1;36m%s\033[0m [\033[0m%s\033[0m] \033[33m%s%%\033[0m  \033[35m%s\033[0m/\033[35m%s\033[0m  \033[32m%s\033[0m  \033[2melapsed %s\033[0m  \033[2mETA %s\033[0m' \
    "$label" "${bar}" "$pct" "$cur_h" "$total_h" "$rate_h" "$(_nlt_pb_fmt_hms "$elapsed")" "$eta")"
  printf '%s\033[K' "$line"
}

nlt_pb_done() {
  [[ -t 1 ]] && printf '\n' || true
}

# nlt_pb_curl_to_file url dest [optional_total_bytes]
# 可选第三参为已知总字节；否则尝试 HEAD 解析 Content-Length。
nlt_pb_curl_to_file() {
  local url="$1" dest="$2" total="${3:-}"
  local start curl_pid ec size last_plain now

  [[ -n "$url" && -n "$dest" ]] || return 2
  command -v curl >/dev/null 2>&1 || return 127

  url="$(_nlt_github_download_resolve_url "$url")"

  if [[ -z "$total" || ! "$total" =~ ^[0-9]+$ ]]; then
    total="$(_nlt_pb_parse_content_length "$url")"
  fi
  [[ ! "$total" =~ ^[0-9]+$ ]] && total=0

  start="$(_nlt_pb_now_s)"
  ec=0
  last_plain=0

  rm -f "$dest"
  curl -fL --connect-timeout 30 -o "$dest" "$url" &
  curl_pid=$!

  if [[ -t 1 ]]; then
    while kill -0 "$curl_pid" 2>/dev/null; do
      size="$(_nlt_file_size "$dest")"
      nlt_pb_render "$size" "$total" "${NLT_PB_LABEL:-download}" "$start"
      sleep 0.25
    done
  else
    while kill -0 "$curl_pid" 2>/dev/null; do
      size="$(_nlt_file_size "$dest")"
      now="$(_nlt_pb_now_s)"
      if [[ $((now - last_plain)) -ge 5 ]]; then
        if [[ "${NONINTERACTIVE:-}" != "1" ]]; then
          if [[ "$total" -gt 0 ]]; then
            printf '%s: %s / %s bytes\n' "${NLT_PB_LABEL:-download}" "$size" "$total" >&2
          else
            printf '%s: %s bytes\n' "${NLT_PB_LABEL:-download}" "$size" >&2
          fi
        fi
        last_plain=$now
      fi
      sleep 0.25
    done
  fi

  wait "$curl_pid" || ec=$?

  size="$(_nlt_file_size "$dest")"
  if [[ "$total" -eq 0 && "$size" -gt 0 ]]; then
    total=$size
  fi
  if [[ -t 1 ]]; then
    nlt_pb_render "$size" "$total" "${NLT_PB_LABEL:-download}" "$start"
    nlt_pb_done
  fi

  return "$ec"
}
