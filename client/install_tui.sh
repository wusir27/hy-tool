#!/usr/bin/env bash
# install_tui.sh — download hy-tui for this OS/arch from wusir27/hy-tool Releases.
# bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/client/install_tui.sh)
#
# HY_TUI_TAG  Release tag (default: latest)
# HY_TUI_DIR  Install directory (default: $HOME/.hy/bin)
# Never sudo, never start the TUI, never download hy, never write yaml.

set -euo pipefail

hy_tui_asset_name() {
  local os arch
  os=$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')
  arch=$(printf '%s' "${2:-}" | tr '[:upper:]' '[:lower:]')

  case "$os" in
    windows|windows_nt|mingw*|msys*|cygwin*)
      echo "hy-tui 不支持 Windows（没有 Windows TUN）。" >&2
      return 1
      ;;
    darwin|macos)
      os=darwin
      ;;
    linux)
      os=linux
      ;;
    *)
      echo "无法识别的系统/架构：${1:-} ${2:-}" >&2
      return 1
      ;;
  esac

  case "$arch" in
    aarch64|arm64)
      arch=arm64
      ;;
    x86_64|amd64)
      arch=amd64
      ;;
    *)
      echo "无法识别的系统/架构：${1:-} ${2:-}" >&2
      return 1
      ;;
  esac

  printf 'hy-tui-%s-%s\n' "$os" "$arch"
}

hy_tui_sha256() {
  local f="$1"
  if [[ "$(uname -s)" == Darwin ]]; then
    shasum -a 256 "$f" | awk '{print $1}'
  else
    sha256sum "$f" | awk '{print $1}'
  fi
}

hy_tui_sums_hash() {
  local sums="$1" name="$2"
  awk -v n="$name" '
    $2 == n || $2 == ("*" n) { print $1; found=1; exit }
    END { if (!found) exit 1 }
  ' "$sums"
}

# Returns 0 on HTTP 200, 2 on 404 / missing file, 1 otherwise.
hy_tui_curl() {
  local url="$1" out="$2" code rc
  set +e
  code=$(curl -fL -A "hy-tui-install" -o "$out" -w '%{http_code}' "$url")
  rc=$?
  set -e
  if [[ $rc -eq 0 ]]; then
    return 0
  fi
  if [[ "$code" == 404 ]]; then
    return 2
  fi
  return 1
}

hy_tui_download_base() {
  if [[ -n "${HY_TUI_BASE:-}" ]]; then
    printf '%s\n' "${HY_TUI_BASE%/}"
  elif [[ -n "${HY_TUI_TAG:-}" ]]; then
    printf 'https://github.com/wusir27/hy-tool/releases/download/%s\n' "$HY_TUI_TAG"
  else
    printf 'https://github.com/wusir27/hy-tool/releases/latest/download\n'
  fi
}

_HY_TUI_TMP=

hy_tui_cleanup() {
  if [[ -n "${_HY_TUI_TMP:-}" ]]; then
    rm -rf "$_HY_TUI_TMP"
    _HY_TUI_TMP=
  fi
}

hy_tui_install() {
  local os arch asset dest dest_dir base expected got staged
  os="${HY_TUI_OS:-$(uname -s)}"
  arch="${HY_TUI_ARCH:-$(uname -m)}"
  asset=$(hy_tui_asset_name "$os" "$arch") || return $?

  dest_dir="${HY_TUI_DIR:-$HOME/.hy/bin}"
  dest="${dest_dir}/hy-tui"
  base=$(hy_tui_download_base)

  command -v curl >/dev/null || {
    echo "需要 curl。" >&2
    return 1
  }

  _HY_TUI_TMP=$(mktemp -d)
  trap hy_tui_cleanup EXIT

  if ! hy_tui_curl "${base}/${asset}" "${_HY_TUI_TMP}/${asset}"; then
    echo "没有对应资产：${asset}" >&2
    return 1
  fi
  if ! hy_tui_curl "${base}/SHA256SUMS" "${_HY_TUI_TMP}/SHA256SUMS"; then
    echo "没有对应资产：SHA256SUMS" >&2
    return 1
  fi

  expected=$(hy_tui_sums_hash "${_HY_TUI_TMP}/SHA256SUMS" "$asset") || {
    echo "没有对应资产：${asset}" >&2
    return 1
  }
  got=$(hy_tui_sha256 "${_HY_TUI_TMP}/${asset}")
  if [[ "$got" != "$expected" ]]; then
    echo "SHA256 校验失败，未覆盖 ${dest}" >&2
    return 1
  fi

  mkdir -p "$dest_dir"
  staged="${dest_dir}/.hy-tui.$$.tmp"
  cp "${_HY_TUI_TMP}/${asset}" "$staged"
  chmod 755 "$staged"
  mv "$staged" "$dest"
  chmod 755 "$dest"

  echo "已安装：${dest}"
  echo "运行：${dest}"
  echo "TUN 仍需要 sudo。"
  echo "说明：https://github.com/wusir27/hy-tool/blob/main/client/tui.md"
}

if [[ "${BASH_SOURCE[0]:-}" == "$0" ]]; then
  if [[ "${1:-}" == --print-asset ]]; then
    hy_tui_asset_name "${HY_TUI_OS:-$(uname -s)}" "${HY_TUI_ARCH:-$(uname -m)}"
    exit $?
  fi
  hy_tui_install
fi
