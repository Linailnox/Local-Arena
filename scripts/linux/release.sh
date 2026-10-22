#!/usr/bin/env bash
#
# Cross-build the Windows release from Linux by wrapping the existing
# PowerShell pipeline (scripts/package.ps1 -> scripts/build.ps1, which uses
# cargo-xwin + clang-cl for the x86_64-pc-windows-msvc target).
#
#   scripts/linux/release.sh --check                verify toolchain, print the command, run nothing
#   scripts/linux/release.sh --version 1.4.3.3      full build + package into artifacts/
#
# Environment: LLVM_BIN (dir containing clang-cl/lld-link/llvm-rc), PWSH,
# XWIN_CACHE (default .cache/xwin). Missing pieces are reported by --check with
# the exact install hint.
set -uo pipefail
cd "$(dirname "$0")/../.."

MODE="run"
VERSION="${RELEASE_VERSION:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --check) MODE="check"; shift ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    *) echo "unknown argument: $1"; exit 2 ;;
  esac
done

MISSING=0
ok()   { echo "  ok    $1"; }
miss() { echo "  MISS  $1${2:+ — $2}"; MISSING=1; }

find_pwsh() {
  if [ -n "${PWSH:-}" ] && [ -x "$PWSH" ]; then echo "$PWSH"; return 0; fi
  for c in pwsh /tmp/opencode/pwsh/pwsh; do
    if command -v "$c" >/dev/null 2>&1 || [ -x "$c" ]; then echo "$c"; return 0; fi
  done
  return 1
}

find_dotnet() {
  if [ -n "${DOTNET:-}" ]; then echo "$DOTNET"; return 0; fi
  local cand
  for cand in "$HOME/.dotnet/dotnet" "$(command -v dotnet 2>/dev/null || true)"; do
    [ -n "$cand" ] && [ -x "$cand" ] || continue
    if [ -n "$($cand --list-sdks 2>/dev/null)" ]; then echo "$cand"; return 0; fi
  done
  return 1
}

find_tool() {
  local name="$1"
  if [ -n "${LLVM_BIN:-}" ] && [ -x "$LLVM_BIN/$name" ]; then echo "$LLVM_BIN/$name"; return 0; fi
  if [ -x ".cache/toolchains/llvm/bin/$name" ]; then echo ".cache/toolchains/llvm/bin/$name"; return 0; fi
  if command -v "$name" >/dev/null 2>&1; then command -v "$name"; return 0; fi
  return 1
}

echo "=== release toolchain check ==="
PWSH_BIN=$(find_pwsh) && ok "pwsh: $PWSH_BIN" || miss "pwsh" "install PowerShell 7 (system package) or set PWSH=path/to/pwsh"
DOTNET_BIN=$(find_dotnet) && ok "dotnet: $DOTNET_BIN ($($DOTNET_BIN --list-sdks | tail -1))" || miss "dotnet" "install SDK 10 via dot.net/v1/dotnet-install.sh"
command -v cargo  >/dev/null 2>&1 && ok "cargo: $(cargo --version)" || miss "cargo"
command -v rustc  >/dev/null 2>&1 && ok "rustc: $(rustc --version)" || miss "rustc"
command -v rustup >/dev/null 2>&1 && ok "rustup" || miss "rustup" "needed to select the target toolchain"
command -v node   >/dev/null 2>&1 && ok "node: $(node --version)" || miss "node"
command -v bun    >/dev/null 2>&1 && ok "bun: $(bun --version)" || miss "bun" "install bun (https://bun.sh) and put it on PATH"
command -v python3 >/dev/null 2>&1 && ok "python3" || miss "python3" "needed by sign-update.py when CSBIP_UPDATE_SIGNING_KEY is set"
CLANG_CL=$(find_tool clang-cl)   && ok "clang-cl: $CLANG_CL"   || miss "clang-cl" "install LLVM (llvm package ships clang-cl on Linux)"
LLD_LINK=$(find_tool lld-link)   && ok "lld-link: $LLD_LINK"   || miss "lld-link" "install LLVM (lld package)"
LLVM_RC=$(find_tool llvm-rc)     && ok "llvm-rc: $LLVM_RC"     || miss "llvm-rc" "install LLVM"
if command -v cargo-xwin >/dev/null 2>&1 || [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo-xwin" ]; then
  ok "cargo-xwin"
else
  miss "cargo-xwin" "cargo install --locked cargo-xwin"
fi
XWIN_CACHE="${XWIN_CACHE:-.cache/xwin}"
if [ -d "$XWIN_CACHE" ] && [ -n "$(ls -A "$XWIN_CACHE" 2>/dev/null)" ]; then
  ok "xwin cache: $XWIN_CACHE"
else
  miss "xwin sdk cache" "cargo xwin setup (populates $XWIN_CACHE)"
fi

LLVM_DIR="${LLVM_BIN:-}"
if [ -z "$LLVM_DIR" ] && [ -d .cache/toolchains/llvm/bin ]; then LLVM_DIR=".cache/toolchains/llvm/bin"; fi
if [ -z "$LLVM_DIR" ] && command -v clang-cl >/dev/null 2>&1; then LLVM_DIR=$(dirname "$(command -v clang-cl)"); fi

CMD=(scripts/package.ps1)
[ -n "$VERSION" ] && CMD+=(-ReleaseVersion "$VERSION")
[ -n "$LLVM_DIR" ] && CMD+=(-LlvmBin "$LLVM_DIR")
CMD+=(-XwinCache "$XWIN_CACHE" -OutputDirectory artifacts)

echo ""
echo "would run:"
echo "  \$PWSH -NoProfile -File ${CMD[*]}"

if [ "$MODE" = "check" ]; then
  if [ "$MISSING" -ne 0 ]; then echo "CHECK: FAIL"; exit 1; fi
  echo "CHECK: PASS"
  exit 0
fi

if [ "$MISSING" -ne 0 ]; then echo "RESULT: FAIL (toolchain incomplete, see above)"; exit 1; fi
if [ -z "$VERSION" ]; then
  echo "RESULT: FAIL (--version X.Y.Z.W or RELEASE_VERSION is required)"
  exit 2
fi

exec "$PWSH_BIN" -NoProfile -File "${CMD[@]}"
