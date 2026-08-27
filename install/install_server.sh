#!/usr/bin/env bash
# Compat: old raw URL. Canonical script is server/install_server.sh
set -euo pipefail
exec bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/server/install_server.sh) "$@"
