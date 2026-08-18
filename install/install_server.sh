#!/usr/bin/env bash
#
# install_server.sh - hy server install script
# Try `install_server.sh --help` for usage.
#
# SPDX-License-Identifier: MIT
# Copyright (c) 2026 wusir27
#

set -e


###
# SCRIPT CONFIGURATION
###

# Basename of this script
SCRIPT_NAME="$(basename "$0")"

# Command line arguments of this script
SCRIPT_ARGS=("$@")

# Path for installing executable (never /usr/local/bin/hysteria)
EXECUTABLE_INSTALL_PATH="/usr/local/bin/hy"

# Paths to install systemd files
SYSTEMD_SERVICES_DIR="/etc/systemd/system"

# Directory to store hy config file (never /etc/hysteria)
CONFIG_DIR="/etc/hy"

# Default server config file name (never config.yaml)
DEFAULT_CONFIG_NAME="server"

# GitHub repository that publishes hy releases
REPO_OWNER="wusir27"
REPO_NAME="hy"
REPO_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}"
GITHUB_API_RELEASES="https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases"

# Re-fetch URL when this script is running from stdin / process substitution
SCRIPT_SELF_URL="https://raw.githubusercontent.com/wusir27/hy-tool/main/install/install_server.sh"

# curl command line flags.
# To using a proxy, please specify ALL_PROXY in the environ variable, such like:
# export ALL_PROXY=socks5h://192.0.2.1:1080
CURL_FLAGS=(-q -L -f --retry 5 --retry-delay 10 --retry-max-time 60)


###
# AUTO DETECTED GLOBAL VARIABLE
###

# Package manager
PACKAGE_MANAGEMENT_INSTALL="${PACKAGE_MANAGEMENT_INSTALL:-}"

# Operating System of current machine, supported: linux
OPERATING_SYSTEM="${OPERATING_SYSTEM:-}"

# Architecture of current machine, supported: 386, amd64, armv7, arm64
ARCHITECTURE="${ARCHITECTURE:-}"

# User for running hy (do not read HYSTERIA_USER)
HY_USER="${HY_USER:-}"

# Directory for ACME certificates storage / systemd WorkingDirectory fallback
HY_HOME_DIR="${HY_HOME_DIR:-}"

# libc flavor for GitHub assets: gnu (default) or musl
HY_LIBC="${HY_LIBC:-}"

# SELinux context of systemd unit files
SECONTEXT_SYSTEMD_UNIT="${SECONTEXT_SYSTEMD_UNIT:-}"


###
# ARGUMENTS
###

# Supported operation: install, remove, check_update
OPERATION=

# User specified version to install
VERSION=

# Force install even if installed
FORCE=

# User specified binary to install
LOCAL_FILE=


###
# COMMAND REPLACEMENT & UTILITIES
###

has_command() {
  local _command=$1

  type -P "$_command" > /dev/null 2>&1
}

curl() {
  command curl "${CURL_FLAGS[@]}" "$@"
}

mktemp() {
  command mktemp "$@" "/tmp/hyinst.XXXXXXXXXX"
}

tput() {
  if has_command tput; then
    command tput "$@"
  fi
}

tred() {
  tput setaf 1
}

tgreen() {
  tput setaf 2
}

tyellow() {
  tput setaf 3
}

tblue() {
  tput setaf 4
}

taoi() {
  tput setaf 6
}

tbold() {
  tput bold
}

treset() {
  tput sgr0
}

note() {
  local _msg="$1"

  echo -e "$SCRIPT_NAME: $(tbold)note: $_msg$(treset)" >&2
}

warning() {
  local _msg="$1"

  echo -e "$SCRIPT_NAME: $(tyellow)warning: $_msg$(treset)" >&2
}

error() {
  local _msg="$1"

  echo -e "$SCRIPT_NAME: $(tred)error: $_msg$(treset)" >&2
}

has_prefix() {
    local _s="$1"
    local _prefix="$2"

    if [[ -z "$_prefix" ]]; then
        return 0
    fi

    if [[ -z "$_s" ]]; then
        return 1
    fi

    [[ "x$_s" != "x${_s#"$_prefix"}" ]]
}

generate_random_password() {
  dd if=/dev/urandom bs=18 count=1 status=none | base64 | tr -d '\n'
}

systemctl() {
  if [[ "x$FORCE_NO_SYSTEMD" == "x2" ]] || ! has_command systemctl; then
    warning "Ignored systemd command: systemctl $@"
    return
  fi

  command systemctl "$@"
}

chcon() {
  if ! has_command chcon || [[ "x$FORCE_NO_SELINUX" == "x1" ]]; then
    return
  fi

  command chcon "$@"
}

get_systemd_version() {
  if ! has_command systemctl; then
    return
  fi

  command systemctl --version | head -1 | cut -d ' ' -f 2
}

systemd_unit_working_directory() {
  local _systemd_version="$(get_systemd_version || true)"

  # WorkingDirectory=~ requires systemd v227 or later.
  if [[ -n "$_systemd_version" && "$_systemd_version" -lt "227" ]]; then
    echo "$HY_HOME_DIR"
    return
  fi

  echo "~"
}

get_selinux_context() {
  local _file="$1"

  local _lsres="$(ls -dZ "$_file" | head -1)"
  local _sectx=''
  case "$(echo "$_lsres" | wc -w)" in
    2)
      _sectx="$(echo "$_lsres" | cut -d ' ' -f 1)"
      ;;
    5)
      _sectx="$(echo "$_lsres" | cut -d ' ' -f 4)"
      ;;
    *)
      ;;
  esac

  if [[ "x$_sectx" == "x?" ]]; then
    _sectx=""
  fi

  echo "$_sectx"
}

show_argument_error_and_exit() {
  local _error_msg="$1"

  error "$_error_msg"
  echo "Try \"$0 --help\" for usage." >&2
  exit 22
}

install_content() {
  local _install_flags="$1"
  local _content="$2"
  local _destination="$3"
  local _overwrite="$4"

  local _tmpfile="$(mktemp)"

  echo -ne "Install $_destination ... "
  echo "$_content" > "$_tmpfile"
  if [[ -z "$_overwrite" && -e "$_destination" ]]; then
    echo -e "exists"
  elif install "$_install_flags" "$_tmpfile" "$_destination"; then
    echo -e "ok"
  fi

  rm -f "$_tmpfile"
}

remove_file() {
  local _target="$1"

  echo -ne "Remove $_target ... "
  if rm "$_target"; then
    echo -e "ok"
  fi
}

exec_sudo() {
  # exec sudo with configurable environ preserved.
  local _saved_ifs="$IFS"
  IFS=$'\n'
  local _preserved_env=(
    $(env | grep "^PACKAGE_MANAGEMENT_INSTALL=" || true)
    $(env | grep "^OPERATING_SYSTEM=" || true)
    $(env | grep "^ARCHITECTURE=" || true)
    $(env | grep "^HY_\w*=" || true)
    $(env | grep "^SECONTEXT_SYSTEMD_UNIT=" || true)
    $(env | grep "^FORCE_\w*=" || true)
  )
  IFS="$_saved_ifs"

  exec sudo env \
    "${_preserved_env[@]}" \
    "$@"
}

detect_package_manager() {
  if [[ -n "$PACKAGE_MANAGEMENT_INSTALL" ]]; then
    return 0
  fi

  if has_command apt; then
    apt update
    PACKAGE_MANAGEMENT_INSTALL='apt -y --no-install-recommends install'
    return 0
  fi

  if has_command dnf; then
    PACKAGE_MANAGEMENT_INSTALL='dnf -y install'
    return 0
  fi

  if has_command yum; then
    PACKAGE_MANAGEMENT_INSTALL='yum -y install'
    return 0
  fi

  if has_command zypper; then
    PACKAGE_MANAGEMENT_INSTALL='zypper install -y --no-recommends'
    return 0
  fi

  if has_command pacman; then
    PACKAGE_MANAGEMENT_INSTALL='pacman -Syu --noconfirm'
    return 0
  fi

  return 1
}

install_software() {
  local _package_name="$1"

  if ! detect_package_manager; then
    error "Supported package manager is not detected, please install the following package manually:"
    echo
    echo -e "\t* $_package_name"
    echo
    exit 65
  fi

  echo "Installing missing dependence '$_package_name' with '$PACKAGE_MANAGEMENT_INSTALL' ... "
  if $PACKAGE_MANAGEMENT_INSTALL "$_package_name"; then
    echo "ok"
  else
    error "Cannot install '$_package_name' with detected package manager, please install it manually."
    exit 65
  fi
}

is_user_exists() {
  local _user="$1"

  id "$_user" > /dev/null 2>&1
}

is_elf_executable() {
  local _file="$1"
  local _magic

  _magic="$(od -An -N4 -tx1 "$_file" 2>/dev/null | tr -d '[:space:]' | tr 'A-F' 'a-f')"
  [[ "$_magic" == "7f454c46" ]]
}

rerun_with_sudo() {
  if ! has_command sudo; then
    return 13
  fi

  local _target_script

  if has_prefix "$0" "/dev/" || has_prefix "$0" "/proc/"; then
    local _tmp_script="$(mktemp)"
    chmod +x "$_tmp_script"

    if has_command curl; then
      curl -o "$_tmp_script" "$SCRIPT_SELF_URL"
    elif has_command wget; then
      wget -O "$_tmp_script" "$SCRIPT_SELF_URL"
    else
      return 127
    fi

    _target_script="$_tmp_script"
  else
    _target_script="$0"
  fi

  note "Re-running this script with sudo. You can also specify FORCE_NO_ROOT=1 to force this script to run as the current user."
  exec_sudo "$_target_script" "${SCRIPT_ARGS[@]}"
}

check_permission() {
  if [[ "$UID" -eq '0' ]]; then
    return
  fi

  note "The user running this script is not root."

  case "$FORCE_NO_ROOT" in
    '1')
      warning "FORCE_NO_ROOT=1 detected, we will proceed without root, but you may get insufficient privileges errors."
      ;;
    *)
      if ! rerun_with_sudo; then
        error "Please run this script with root or specify FORCE_NO_ROOT=1 to force this script to run as the current user."
        exit 13
      fi
      ;;
  esac
}

check_environment_operating_system() {
  if [[ -n "$OPERATING_SYSTEM" ]]; then
    warning "OPERATING_SYSTEM=$OPERATING_SYSTEM detected, operating system detection will not be performed."
    case "$OPERATING_SYSTEM" in
      linux)
        return
        ;;
      darwin|windows|freebsd)
        error "This script only supports Linux (OPERATING_SYSTEM=$OPERATING_SYSTEM is not supported)."
        exit 95
        ;;
      *)
        error "This script only supports Linux (OPERATING_SYSTEM=$OPERATING_SYSTEM is not supported)."
        exit 95
        ;;
    esac
  fi

  if [[ "x$(uname)" == "xLinux" ]]; then
    OPERATING_SYSTEM=linux
    return
  fi

  error "This script only supports Linux."
  exit 95
}

assert_supported_architecture() {
  case "$ARCHITECTURE" in
    'amd64' | 'arm64' | 'armv7' | '386')
      return 0
      ;;
    'amd64-avx')
      error "hy does not publish an amd64-avx build. Use ARCHITECTURE=amd64 (asset hy-linux-amd64)."
      exit 8
      ;;
    'arm')
      error "hy uses architecture 'armv7' (asset hy-linux-armv7), not official Hysteria's 'arm'."
      exit 8
      ;;
    'darwin' | 'windows' | 'mips' | 'mipsle' | 'mips64' | 'mips64le' | 's390x' | 'loong64')
      error "Architecture '$ARCHITECTURE' is not supported."
      note "This script only installs Linux gnu binaries: hy-linux-amd64, hy-linux-arm64, hy-linux-armv7, hy-linux-386."
      exit 8
      ;;
    *)
      error "Architecture '$ARCHITECTURE' is not supported."
      note "This script only installs Linux gnu binaries: hy-linux-amd64, hy-linux-arm64, hy-linux-armv7, hy-linux-386."
      exit 8
      ;;
  esac
}

check_environment_architecture() {
  if [[ -n "$ARCHITECTURE" ]]; then
    warning "ARCHITECTURE=$ARCHITECTURE detected, architecture detection will not be performed."
    assert_supported_architecture
    return
  fi

  case "$(uname -m)" in
    i386 | i686)
      ARCHITECTURE='386'
      ;;
    amd64 | x86_64)
      ARCHITECTURE='amd64'
      ;;
    aarch64 | arm64)
      ARCHITECTURE='arm64'
      ;;
    armv7l | armv7* | armv6* | armv5*)
      ARCHITECTURE='armv7'
      ;;
    mips | mipsle | mips64 | mips64le | s390x | loongarch64 | loong64)
      error "The architecture '$(uname -m)' is not supported."
      note "This script only installs Linux gnu binaries: hy-linux-amd64, hy-linux-arm64, hy-linux-armv7, hy-linux-386."
      exit 8
      ;;
    *)
      error "The architecture '$(uname -m)' is not supported."
      note "This script only installs Linux gnu binaries: hy-linux-amd64, hy-linux-arm64, hy-linux-armv7, hy-linux-386."
      note "Specify ARCHITECTURE=<amd64|arm64|armv7|386> to override detection."
      exit 8
      ;;
  esac
}

check_environment_libc() {
  case "${HY_LIBC:-gnu}" in
    gnu | musl)
      ;;
    *)
      error "HY_LIBC must be 'gnu' or 'musl' (got '$HY_LIBC')."
      exit 8
      ;;
  esac
}

check_environment_systemd() {
  if [[ -d "/run/systemd/system" ]] || grep -q systemd <(ls -l /sbin/init 2>/dev/null || true); then
    return
  fi

  case "$FORCE_NO_SYSTEMD" in
    '1')
      warning "FORCE_NO_SYSTEMD=1, we will proceed as normal even if systemd is not detected."
      ;;
    '2')
      warning "FORCE_NO_SYSTEMD=2, we will proceed but skip all systemd related commands."
      ;;
    *)
      error "This script only supports Linux distributions with systemd."
      note "Specify FORCE_NO_SYSTEMD=1 to disable this check and force this script to run as if systemd exists."
      note "Specify FORCE_NO_SYSTEMD=2 to disable this check and skip all systemd related commands."
      exit 95
      ;;
  esac
}

check_environment_selinux() {
  if ! has_command getenforce; then
    return
  fi

  note "SELinux is detected"

  if [[ "x$FORCE_NO_SELINUX" == "x1" ]]; then
    warning "FORCE_NO_SELINUX=1, we will skip all SELinux related commands."
    return
  fi

  if [[ -z "$SECONTEXT_SYSTEMD_UNIT" ]]; then
    if [[ -z "$FORCE_NO_SYSTEMD" ]] && [[ -e "$SYSTEMD_SERVICES_DIR" ]]; then
      local _sectx="$(get_selinux_context "$SYSTEMD_SERVICES_DIR")"
      if [[ -z "$_sectx" ]]; then
        warning "Failed to obtain SEContext of $SYSTEMD_SERVICES_DIR"
      else
        SECONTEXT_SYSTEMD_UNIT="$_sectx"
      fi
    fi
  fi
}

check_environment_curl() {
  if has_command curl; then
    return
  fi

  install_software curl
}

check_environment_grep() {
  if has_command grep; then
    return
  fi

  install_software grep
}

check_environment() {
  check_environment_operating_system
  check_environment_architecture
  check_environment_libc
  check_environment_systemd
  check_environment_selinux
  check_environment_curl
  check_environment_grep
}

vercmp_segment() {
  local _lhs="$1"
  local _rhs="$2"

  if [[ "x$_lhs" == "x$_rhs" ]]; then
    echo 0
    return
  fi
  if [[ -z "$_lhs" ]]; then
    echo -1
    return
  fi
  if [[ -z "$_rhs" ]]; then
    echo 1
    return
  fi

  local _lhs_num="${_lhs//[A-Za-z]*/}"
  local _rhs_num="${_rhs//[A-Za-z]*/}"

  if [[ "x$_lhs_num" == "x$_rhs_num" ]]; then
    echo 0
    return
  fi
  if [[ -z "$_lhs_num" ]]; then
    echo -1
    return
  fi
  if [[ -z "$_rhs_num" ]]; then
    echo 1
    return
  fi
  local _numcmp=$(($_lhs_num - $_rhs_num))
  if [[ "$_numcmp" -ne 0 ]]; then
    echo "$_numcmp"
    return
  fi

  local _lhs_suffix="${_lhs#"$_lhs_num"}"
  local _rhs_suffix="${_rhs#"$_rhs_num"}"

  if [[ "x$_lhs_suffix" == "x$_rhs_suffix" ]]; then
    echo 0
    return
  fi
  if [[ -z "$_lhs_suffix" ]]; then
    echo 1
    return
  fi
  if [[ -z "$_rhs_suffix" ]]; then
    echo -1
    return
  fi
  if [[ "$_lhs_suffix" < "$_rhs_suffix" ]]; then
    echo -1
    return
  fi
  echo 1
}

vercmp() {
  local _lhs=${1#v}
  local _rhs=${2#v}

  while [[ -n "$_lhs" && -n "$_rhs" ]]; do
    local _clhs="${_lhs/.*/}"
    local _crhs="${_rhs/.*/}"

    local _segcmp="$(vercmp_segment "$_clhs" "$_crhs")"
    if [[ "$_segcmp" -ne 0 ]]; then
      echo "$_segcmp"
      return
    fi

    _lhs="${_lhs#"$_clhs"}"
    _lhs="${_lhs#.}"
    _rhs="${_rhs#"$_crhs"}"
    _rhs="${_rhs#.}"
  done

  if [[ "x$_lhs" == "x$_rhs" ]]; then
    echo 0
    return
  fi

  if [[ -z "$_lhs" ]]; then
    echo -1
    return
  fi

  if [[ -z "$_rhs" ]]; then
    echo 1
    return
  fi

  return
}

check_hy_user() {
  local _default_hy_user="$1"

  if [[ -n "$HY_USER" ]]; then
    return
  fi

  if [[ ! -e "$SYSTEMD_SERVICES_DIR/hy-server.service" ]]; then
    HY_USER="$_default_hy_user"
    return
  fi

  HY_USER="$(grep -o '^User=\w*' "$SYSTEMD_SERVICES_DIR/hy-server.service" | tail -1 | cut -d '=' -f 2 || true)"

  if [[ -z "$HY_USER" ]]; then
    HY_USER="$_default_hy_user"
  fi
}

check_hy_homedir() {
  local _default_hy_homedir="$1"

  if [[ -n "$HY_HOME_DIR" ]]; then
    return
  fi

  if ! is_user_exists "$HY_USER"; then
    HY_HOME_DIR="$_default_hy_homedir"
    return
  fi

  HY_HOME_DIR="$(eval echo ~"$HY_USER")"
}

warn_if_hy_user_is_official() {
  if [[ "x$HY_USER" == "xhysteria" ]]; then
    warning "HY_USER=hysteria is set explicitly; home and ACME directories may overlap with official Hysteria. Proceeding as requested."
  fi
}

note_official_hysteria_if_present() {
  if [[ -f "/usr/local/bin/hysteria" || -h "/usr/local/bin/hysteria" ]]; then
    note "Official Hysteria is present at /usr/local/bin/hysteria and was not changed."
    note "Official config (if any) stays at /etc/hysteria/config.yaml; this script does not write that path."
  fi
}


###
# ARGUMENTS PARSER
###

show_usage_and_exit() {
  echo
  echo -e "\t$(tbold)$SCRIPT_NAME$(treset) - hy server install script"
  echo
  echo -e "Usage:"
  echo
  echo -e "$(tbold)Install hy$(treset)"
  echo -e "\t$0 [ -f | -l <file> | --version <version> ]"
  echo -e "Flags:"
  echo -e "\t-f, --force\tForce re-install latest or specified version even if it has been installed."
  echo -e "\t-l, --local <file>\tInstall specified hy binary instead of download it."
  echo -e "\t--version <version>\tInstall specified version instead of the latest (v0.3.10 or 0.3.10)."
  echo
  echo -e "$(tbold)Remove hy$(treset)"
  echo -e "\t$0 --remove"
  echo
  echo -e "$(tbold)Check for the update$(treset)"
  echo -e "\t$0 -c"
  echo -e "\t$0 --check"
  echo
  echo -e "$(tbold)Show this help$(treset)"
  echo -e "\t$0 -h"
  echo -e "\t$0 --help"
  echo
  echo -e "Environment:"
  echo -e "\tHY_USER, HY_HOME_DIR, ARCHITECTURE, HY_LIBC=gnu|musl"
  echo -e "\tFORCE_NO_ROOT, FORCE_NO_SYSTEMD, FORCE_NO_SELINUX, ALL_PROXY"
  exit 0
}

parse_arguments() {
  while [[ "$#" -gt '0' ]]; do
    case "$1" in
      '--remove')
        if [[ -n "$OPERATION" && "$OPERATION" != 'remove' ]]; then
          show_argument_error_and_exit "Option '--remove' is in conflict with other options."
        fi
        OPERATION='remove'
        ;;
      '--version')
        VERSION="$2"
        if [[ -z "$VERSION" ]]; then
          show_argument_error_and_exit "Please specify the version for option '--version'."
        fi
        shift
        ;;
      '-c' | '--check')
        if [[ -n "$OPERATION" && "$OPERATION" != 'check_update' ]]; then
          show_argument_error_and_exit "Option '-c' or '--check' is in conflict with other options."
        fi
        OPERATION='check_update'
        ;;
      '-f' | '--force')
        FORCE='1'
        ;;
      '-h' | '--help')
        show_usage_and_exit
        ;;
      '-l' | '--local')
        LOCAL_FILE="$2"
        if [[ -z "$LOCAL_FILE" ]]; then
          show_argument_error_and_exit "Please specify the local binary to install for option '-l' or '--local'."
        fi
        shift
        ;;
      *)
        show_argument_error_and_exit "Unknown option '$1'"
        ;;
    esac
    shift
  done

  if [[ -z "$OPERATION" ]]; then
    OPERATION='install'
  fi

  # validate arguments
  case "$OPERATION" in
    'install')
      if [[ -n "$VERSION" && -n "$LOCAL_FILE" ]]; then
        show_argument_error_and_exit '--version and --local cannot be used together.'
      fi
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        show_argument_error_and_exit "--version is only valid for install operation."
      fi
      if [[ -n "$LOCAL_FILE" ]]; then
        show_argument_error_and_exit "--local is only valid for install operation."
      fi
      ;;
  esac
}


###
# FILE TEMPLATES
###

# /etc/systemd/system/hy-server.service
tpl_hy_server_service_base() {
  local _config_name="$1"

  cat << EOF
[Unit]
Description=hy Server Service (${_config_name}.yaml)
After=network.target

[Service]
Type=simple
ExecStart=$EXECUTABLE_INSTALL_PATH server --config ${CONFIG_DIR}/${_config_name}.yaml
WorkingDirectory=$(systemd_unit_working_directory)
User=$HY_USER
Group=$HY_USER
Environment=HYSTERIA_LOG_LEVEL=info
Environment=HYSTERIA_ACME_DIR=$HY_HOME_DIR/acme
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_BIND_SERVICE CAP_NET_RAW
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_BIND_SERVICE CAP_NET_RAW
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
EOF
}

# /etc/systemd/system/hy-server.service
tpl_hy_server_service() {
  tpl_hy_server_service_base "$DEFAULT_CONFIG_NAME"
}

# /etc/systemd/system/hy-server@.service
tpl_hy_server_x_service() {
  tpl_hy_server_service_base '%i'
}

# /etc/hy/server.yaml
tpl_etc_hy_server_yaml() {
  cat << EOF
# listen: :443

acme:
  domains:
    - your.domain.net
  email: your@email.com
  ca: letsencrypt
  type: http

auth:
  type: password
  password: $(generate_random_password)

masquerade:
  type: proxy
  proxy:
    url: https://news.ycombinator.com/
    rewriteHost: true
EOF
}


###
# SYSTEMD
###

get_running_services() {
  if [[ "x$FORCE_NO_SYSTEMD" == "x2" ]]; then
    return
  fi

  # Filter MUST be hy-server so hysteria-server units are never matched.
  systemctl list-units --type=service --state=active --plain --no-legend \
    | awk '{print $1}' \
    | grep -E '^hy-server(@.+)?\.service$' || true
}

restart_running_services() {
  if [[ "x$FORCE_NO_SYSTEMD" == "x2" ]]; then
    return
  fi

  echo "Restarting running service ... "

  for service in $(get_running_services); do
    echo -ne "Restarting $service ... "
    systemctl restart "$service"
    echo "done"
  done
}

stop_running_services() {
  if [[ "x$FORCE_NO_SYSTEMD" == "x2" ]]; then
    return
  fi

  echo "Stopping running service ... "

  for service in $(get_running_services); do
    echo -ne "Stopping $service ... "
    systemctl stop "$service"
    echo "done"
  done
}


###
# HY GITHUB API
###

no_release_error() {
  error "No GitHub Release found for ${REPO_OWNER}/${REPO_NAME}."
  note "Publish a release, or install a local binary with --local FILE."
}

github_api_get() {
  local _url="$1"
  local _out="$2"
  local _code

  _code="$(command curl -q -sS -L --retry 5 --retry-delay 10 --retry-max-time 60 \
    -w '%{http_code}' -o "$_out" "$_url" || true)"
  echo "$_code"
}

parse_json_string_field() {
  local _file="$1"
  local _field="$2"

  sed -n "s/.*\"${_field}\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$_file" | head -1
}

release_has_asset() {
  local _file="$1"
  local _name="$2"

  grep -q "\"name\"[[:space:]]*:[[:space:]]*\"${_name}\"" "$_file"
}

fetch_release_json() {
  local _url="$1"
  local _out="$2"
  local _code

  _code="$(github_api_get "$_url" "$_out")"

  if [[ "$_code" == "404" ]]; then
    if [[ "$_url" == *"/latest" ]]; then
      no_release_error
    else
      error "GitHub Release not found for ${REPO_OWNER}/${REPO_NAME} (HTTP 404)."
      note "Check the tag, or install a local binary with --local FILE."
    fi
    rm -f "$_out"
    exit 11
  fi

  if [[ "$_code" != "200" ]]; then
    error "Failed to query GitHub Releases (HTTP ${_code:-000})."
    note "If you cannot reach GitHub, install with --local FILE."
    rm -f "$_out"
    exit 11
  fi
}

hy_asset_name() {
  local _libc="${HY_LIBC:-gnu}"
  local _name="hy-linux-${ARCHITECTURE}"

  case "$_libc" in
    musl)
      echo "${_name}-musl"
      ;;
    *)
      echo "$_name"
      ;;
  esac
}

ensure_musl_asset_exists() {
  local _tag="$1"
  local _asset="$2"
  local _tmpfile
  local _url

  if [[ "${HY_LIBC:-gnu}" != "musl" ]]; then
    return 0
  fi

  _tmpfile="$(mktemp)"
  if [[ -n "$VERSION" ]]; then
    _url="${GITHUB_API_RELEASES}/tags/${_tag}"
  else
    _url="${GITHUB_API_RELEASES}/latest"
  fi

  fetch_release_json "$_url" "$_tmpfile"

  if ! release_has_asset "$_tmpfile" "$_asset"; then
    error "HY_LIBC=musl requested, but release '${_tag}' does not contain asset '${_asset}'."
    note "This script will not invent a musl filename. Use HY_LIBC=gnu or --local FILE."
    rm -f "$_tmpfile"
    exit 11
  fi

  rm -f "$_tmpfile"
}

is_hy_installed() {
  # RETURN VALUE
  # 0: hy is installed
  # 1: hy is not installed

  if [[ -f "$EXECUTABLE_INSTALL_PATH" || -h "$EXECUTABLE_INSTALL_PATH" ]]; then
    return 0
  fi
  return 1
}

get_installed_version() {
  if ! is_hy_installed; then
    return
  fi

  local _line
  _line="$("$EXECUTABLE_INSTALL_PATH" version 2>/dev/null | head -n 1 || true)"
  _line="${_line#"${_line%%[![:space:]]*}"}"
  _line="${_line%"${_line##*[![:space:]]}"}"
  _line="${_line#v}"
  echo "$_line"
}

get_latest_version() {
  if [[ -n "$VERSION" ]]; then
    echo "$VERSION"
    return
  fi

  local _tmpfile
  local _latest_version

  _tmpfile="$(mktemp)"
  fetch_release_json "${GITHUB_API_RELEASES}/latest" "$_tmpfile"

  _latest_version="$(parse_json_string_field "$_tmpfile" "tag_name")"
  rm -f "$_tmpfile"

  if [[ -z "$_latest_version" ]]; then
    error "GitHub latest release has no tag_name."
    note "Install a local binary with --local FILE."
    exit 11
  fi

  echo "$_latest_version"
}

download_hy() {
  local _version="$1"
  local _destination="$2"
  local _asset
  local _download_url

  _asset="$(hy_asset_name)"
  ensure_musl_asset_exists "$_version" "$_asset"

  _download_url="$REPO_URL/releases/download/${_version}/${_asset}"
  echo "Downloading hy binary: $_download_url ..."
  if ! curl -R -H 'Cache-Control: no-cache' "$_download_url" -o "$_destination"; then
    error "Download failed, please check your network and try again."
    return 11
  fi

  if ! is_elf_executable "$_destination"; then
    error "Downloaded file is not an ELF executable (not a hy Linux binary)."
    return 11
  fi

  return 0
}

check_update() {
  # RETURN VALUE
  # 0: update available
  # 1: installed version is latest

  local _installed_version
  local _latest_version
  local _vercmp

  echo -ne "Checking for installed version ... "
  _installed_version="$(get_installed_version || true)"
  if [[ -n "$_installed_version" ]]; then
    echo "$_installed_version"
  else
    echo "not installed"
  fi

  echo -ne "Checking for latest version ... "
  # Do not use `local x=$(...)`: bash `local` swallows a failing substitution
  # (including `exit` in the subshell), which would treat "no release" as success.
  _latest_version="$(get_latest_version)" || exit $?
  if [[ -n "$_latest_version" ]]; then
    echo "$_latest_version"
    VERSION="$_latest_version"
  else
    echo "failed"
    error "Failed to determine the latest GitHub Release."
    note "Install a local binary with --local FILE."
    exit 11
  fi

  _vercmp="$(vercmp "$_installed_version" "$_latest_version")"
  if [[ "$_vercmp" -lt 0 ]]; then
    return 0
  fi

  return 1
}


###
# ENTRY
###

perform_install_hy_binary() {
  if [[ -n "$LOCAL_FILE" ]]; then
    note "Performing local install: $LOCAL_FILE"

    if [[ ! -f "$LOCAL_FILE" ]]; then
      error "Local file '$LOCAL_FILE' does not exist."
      exit 2
    fi

    if ! is_elf_executable "$LOCAL_FILE"; then
      error "Local file '$LOCAL_FILE' is not an ELF executable."
      exit 2
    fi

    echo -ne "Installing hy executable ... "

    if install -Dm755 "$LOCAL_FILE" "$EXECUTABLE_INSTALL_PATH"; then
      echo "ok"
    else
      exit 2
    fi

    return
  fi

  local _tmpfile
  _tmpfile="$(mktemp)"

  if ! download_hy "$VERSION" "$_tmpfile"; then
    rm -f "$_tmpfile"
    exit 11
  fi

  echo -ne "Installing hy executable ... "

  if install -Dm755 "$_tmpfile" "$EXECUTABLE_INSTALL_PATH"; then
    echo "ok"
  else
    exit 13
  fi

  rm -f "$_tmpfile"
}

perform_remove_hy_binary() {
  remove_file "$EXECUTABLE_INSTALL_PATH"
}

perform_install_hy_example_config() {
  install_content -Dm644 "$(tpl_etc_hy_server_yaml)" "$CONFIG_DIR/${DEFAULT_CONFIG_NAME}.yaml" ""
}

perform_install_hy_systemd() {
  if [[ "x$FORCE_NO_SYSTEMD" == "x2" ]]; then
    return
  fi

  install_content -Dm644 "$(tpl_hy_server_service)" "$SYSTEMD_SERVICES_DIR/hy-server.service" "1"
  install_content -Dm644 "$(tpl_hy_server_x_service)" "$SYSTEMD_SERVICES_DIR/hy-server@.service" "1"
  if [[ -n "$SECONTEXT_SYSTEMD_UNIT" ]]; then
    chcon "$SECONTEXT_SYSTEMD_UNIT" "$SYSTEMD_SERVICES_DIR/hy-server.service"
    chcon "$SECONTEXT_SYSTEMD_UNIT" "$SYSTEMD_SERVICES_DIR/hy-server@.service"
  fi

  systemctl daemon-reload
}

perform_remove_hy_systemd() {
  remove_file "$SYSTEMD_SERVICES_DIR/hy-server.service"
  remove_file "$SYSTEMD_SERVICES_DIR/hy-server@.service"

  systemctl daemon-reload
}

perform_install_hy_user() {
  if ! is_user_exists "$HY_USER"; then
    echo -ne "Creating user $HY_USER ... "
    useradd -r -d "$HY_HOME_DIR" -m "$HY_USER"
    echo "ok"
  fi
}

perform_install() {
  local _is_fresh_install
  if ! is_hy_installed; then
    _is_fresh_install=1
  fi

  local _is_update_required

  if [[ -n "$LOCAL_FILE" ]] || [[ -n "$VERSION" ]] || check_update; then
    _is_update_required=1
  fi

  if [[ "x$FORCE" == "x1" ]]; then
    if [[ -z "$_is_update_required" ]]; then
      note "Option '--force' detected, re-install even if installed version is the latest."
    fi
    _is_update_required=1
  fi

  if [[ -n "$_is_update_required" ]]; then
    perform_install_hy_binary
  fi

  # Always install additional files, regardless of $_is_update_required.
  # This allows changes to be made with environment variables (e.g. change HY_USER without --force).
  perform_install_hy_example_config
  perform_install_hy_user
  perform_install_hy_systemd

  note_official_hysteria_if_present

  if [[ -z "$_is_update_required" ]]; then
    echo
    echo "$(tgreen)Installed version is up-to-date, there is nothing to do.$(treset)"
    echo
  elif [[ -n "$_is_fresh_install" ]]; then
    echo
    echo -e "$(tbold)Congratulation! hy has been successfully installed on your server.$(treset)"
    echo
    echo -e "What's next?"
    echo
    echo -e "\t+ Edit server config file at $(tred)$CONFIG_DIR/${DEFAULT_CONFIG_NAME}.yaml$(treset)"
    echo -e "\t+ Start and enable on boot with $(tred)systemctl enable --now hy-server.service$(treset)"
    echo -e "\t+ Follow USAGE at $(tblue)https://github.com/wusir27/hy/blob/main/USAGE.md$(treset)"
    echo
    echo -e "This script does not enable or start the service automatically."
    echo
  else
    restart_running_services

    echo
    if [[ -n "$VERSION" ]]; then
      echo -e "$(tbold)hy has been successfully updated to $VERSION.$(treset)"
    else
      echo -e "$(tbold)hy has been successfully updated.$(treset)"
    fi
    echo
    echo -e "Changelog: $(tblue)$REPO_URL/releases$(treset)"
    echo
  fi
}

perform_remove() {
  perform_remove_hy_binary
  stop_running_services
  perform_remove_hy_systemd

  note_official_hysteria_if_present

  echo
  echo -e "$(tbold)Congratulation! hy has been successfully removed from your server.$(treset)"
  echo
  echo -e "You still need to remove configuration files and ACME certificates manually with the following commands:"
  echo
  echo -e "\t$(tred)rm -rf "$CONFIG_DIR"$(treset)"
  if [[ "x$HY_USER" != "xroot" ]]; then
    echo -e "\t$(tred)userdel -r "$HY_USER"$(treset)"
  fi
  if [[ "x$FORCE_NO_SYSTEMD" != "x2" ]]; then
    echo
    echo -e "You still might need to disable all related systemd services with the following commands:"
    echo
    echo -e "\t$(tred)rm -f /etc/systemd/system/multi-user.target.wants/hy-server.service$(treset)"
    echo -e "\t$(tred)rm -f /etc/systemd/system/multi-user.target.wants/hy-server@*.service$(treset)"
    echo -e "\t$(tred)systemctl daemon-reload$(treset)"
  fi
  echo
}

perform_check_update() {
  if check_update; then
    echo
    echo -e "$(tbold)Update available: $VERSION$(treset)"
    echo
    echo -e "$(tgreen)You can download and install the latest version by execute this script without any arguments.$(treset)"
    echo
  else
    echo
    echo "$(tgreen)Installed version is up-to-date.$(treset)"
    echo
  fi
}

main() {
  parse_arguments "$@"

  check_permission
  check_environment
  check_hy_user "hy"
  check_hy_homedir "/var/lib/$HY_USER"
  warn_if_hy_user_is_official

  case "$OPERATION" in
    "install")
      perform_install
      ;;
    "remove")
      perform_remove
      ;;
    "check_update")
      perform_check_update
      ;;
    *)
      error "Unknown operation '$OPERATION'."
      exit 64
      ;;
  esac
}

main "$@"

# vim:set ft=bash ts=2 sw=2 sts=2 et:
