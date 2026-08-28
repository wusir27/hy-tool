#!/usr/bin/env bash
# Tests for client/install_tui.sh (mapping + live Linux install). Does not start the TUI.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/client/install_tui.sh"
pass=0
fail=0

ok() {
  local name="$1"
  shift
  if "$@"; then
    echo "PASS  $name"
    pass=$((pass + 1))
  else
    echo "FAIL  $name" >&2
    fail=$((fail + 1))
  fi
}

eq() {
  test "$1" = "$2"
}

contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) return 1 ;;
  esac
}

print_asset() {
  HY_TUI_OS="$1" HY_TUI_ARCH="$2" bash "$SCRIPT" --print-asset
}

echo "== bash -n"
bash -n "$SCRIPT"
ok "bash -n install_tui.sh" true

echo "== OS/arch mapping"
got=$(print_asset Linux x86_64)
ok "linux x86_64 → hy-tui-linux-amd64" eq "$got" hy-tui-linux-amd64

got=$(print_asset linux amd64)
ok "linux amd64 → hy-tui-linux-amd64" eq "$got" hy-tui-linux-amd64

got=$(print_asset Darwin arm64)
ok "darwin arm64 → hy-tui-darwin-arm64" eq "$got" hy-tui-darwin-arm64

got=$(print_asset darwin aarch64)
ok "darwin aarch64 → hy-tui-darwin-arm64" eq "$got" hy-tui-darwin-arm64

got=$(print_asset Darwin x86_64)
ok "darwin x86_64 → hy-tui-darwin-amd64" eq "$got" hy-tui-darwin-amd64

got=$(print_asset linux aarch64)
ok "linux aarch64 → hy-tui-linux-arm64" eq "$got" hy-tui-linux-arm64

echo "== Darwin mapping never returns a linux asset"
darwin_ok=1
for os in Darwin darwin macOS macos; do
  for arch in arm64 aarch64 x86_64 amd64; do
    name=$(print_asset "$os" "$arch")
    case "$name" in
      *linux*)
        echo "  bad: $os/$arch → $name (contains linux)" >&2
        darwin_ok=0
        ;;
      hy-tui-darwin-arm64|hy-tui-darwin-amd64) ;;
      *)
        echo "  bad: $os/$arch → $name (not darwin)" >&2
        darwin_ok=0
        ;;
    esac
  done
done
ok "Darwin mapping stays on hy-tui-darwin-*" eq "$darwin_ok" 1

echo "== unknown OS/arch → non-zero"
set +e
err=$(print_asset freebsd x86_64 2>&1)
rc=$?
set -e
ok "freebsd/x86_64 exits non-zero" test "$rc" -ne 0
ok "freebsd/x86_64 prints to stderr" test -n "$err"

set +e
err=$(print_asset linux riscv64 2>&1)
rc=$?
set -e
ok "linux/riscv64 exits non-zero" test "$rc" -ne 0

set +e
err=$(print_asset linux armv7 2>&1)
rc=$?
set -e
ok "linux/armv7 exits non-zero (no armv7 asset)" test "$rc" -ne 0

set +e
err=$(print_asset Windows x86_64 2>&1)
rc=$?
set -e
ok "Windows exits non-zero" test "$rc" -ne 0

echo "== live install (tui-v0.1.0 linux-amd64)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
dest_dir="$tmp/bin"
set +e
out=$(HY_TUI_TAG=tui-v0.1.0 HY_TUI_DIR="$dest_dir" bash "$SCRIPT" 2>&1)
rc=$?
set -e
ok "live install exit 0" eq "$rc" 0
ok "live install wrote dest" test -f "$dest_dir/hy-tui"
mode=$(stat -c '%a' "$dest_dir/hy-tui")
ok "dest mode 0755" eq "$mode" 755
want=$(curl -fsSL https://github.com/wusir27/hy-tool/releases/download/tui-v0.1.0/SHA256SUMS | awk '$2=="hy-tui-linux-amd64"{print $1}')
got=$(sha256sum "$dest_dir/hy-tui" | awk '{print $1}')
ok "live sha matches SHA256SUMS" eq "$got" "$want"
ok "success mentions dest path" contains "$out" "$dest_dir/hy-tui"
ok "success mentions sudo" contains "$out" sudo

good_hash=$got
fixture="$tmp/fixture"
mkdir -p "$fixture"
cp "$dest_dir/hy-tui" "$fixture/hy-tui-linux-amd64"
printf '%s  %s\n' "0000000000000000000000000000000000000000000000000000000000000000" "hy-tui-linux-amd64" >"$fixture/SHA256SUMS"

echo "== flipped SHA256SUMS does not overwrite"
set +e
out=$(HY_TUI_OS=Linux HY_TUI_ARCH=x86_64 HY_TUI_BASE="file://${fixture}" HY_TUI_DIR="$dest_dir" bash "$SCRIPT" 2>&1)
rc=$?
set -e
ok "flipped SHA exits non-zero" test "$rc" -ne 0
after=$(sha256sum "$dest_dir/hy-tui" | awk '{print $1}')
ok "flipped SHA left dest intact" eq "$after" "$good_hash"
mode=$(stat -c '%a' "$dest_dir/hy-tui")
ok "flipped SHA left mode 0755" eq "$mode" 755

echo "== missing Darwin asset"
set +e
out=$(HY_TUI_OS=Darwin HY_TUI_ARCH=arm64 HY_TUI_TAG=tui-v0.1.0 HY_TUI_DIR="$tmp/darwin-bin" bash "$SCRIPT" 2>&1)
rc=$?
set -e
ok "missing Darwin exits non-zero" test "$rc" -ne 0
ok "missing Darwin prints 没有对应资产" contains "$out" "没有对应资产"
ok "missing Darwin did not write dest" test ! -e "$tmp/darwin-bin/hy-tui"

echo
echo "passed=$pass failed=$fail"
if test "$fail" -ne 0; then
  exit 1
fi
