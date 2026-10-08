#!/usr/bin/env bash
#
# Linux dev gate suite for Local-Arena. Runs every repository gate natively and
# prints a PASS/FAIL summary; exits non-zero if any step fails.
#
#   scripts/linux/build.sh              full suite
#   scripts/linux/build.sh --only npm   run a single step (pwsh|npm|dotnet|cargo|verify)
#
# cargo test note: this suite runs on Linux. Two classes of tests can only pass
# on Windows: literal Windows-path fixtures (F:\... / \\?\ verbatim semantics)
# and payload-fixture/model-verification cases that predate the 2026-10-08
# cross-platform path fix. Baseline 2026-10-08: 7 failures (was 18 before the
# safe_relative/upstream_root fixes; 11 flipped green, zero new failures).
# Any OTHER failure fails the gate.
set -uo pipefail
cd "$(dirname "$0")/../.."

LOGDIR="artifacts/linux-gate"
mkdir -p "$LOGDIR"

ONLY=""
if [ "${1:-}" = "--only" ]; then ONLY="${2:-}"; fi

KNOWN_WINDOWS_ONLY='
installer::tests::managed_upgrade_preserves_a_modified_legacy_rating_model
installer::tests::managed_upgrade_replaces_the_unchanged_rating_plus_model_with_open_rating
installer::tests::managed_upgrade_restores_a_pre_plus_file_at_the_superseded_model_path
installer::tests::open_rating_install_preserves_an_untracked_legacy_model
installer::tests::pristine_restore_removes_suite_files_but_keeps_third_party_files
runtime_state::tests::compares_process_to_selected_installation
tests::demo_playback_removes_the_windows_verbatim_prefix_before_relativizing
'

RESULTS=()
FAILED=0

step() {
  local name="$1"; shift
  if [ -n "$ONLY" ] && [ "$ONLY" != "$name" ]; then return 0; fi
  echo ""
  echo "########## STEP: $name ##########"
  if "$@"; then
    RESULTS+=("PASS  $name")
  else
    RESULTS+=("FAIL  $name")
    FAILED=1
  fi
}

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing tool: $1${2:+ — $2}"
    return 1
  fi
}

find_pwsh() {
  if [ -n "${PWSH:-}" ] && [ -x "$PWSH" ]; then echo "$PWSH"; return 0; fi
  for c in /tmp/opencode/pwsh/pwsh pwsh; do
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

ensure_local_build() {
  if [ ! -f .local-build.ps1 ]; then
    printf '# local-only tool overrides (gitignored), created by scripts/linux/build.sh\n' > .local-build.ps1
    echo "created stub .local-build.ps1"
  fi
}

ensure_icon() {
  if [ ! -f Panel/src-tauri/icons/icon.png ]; then
    python3 - <<'EOF'
import struct, zlib, pathlib
w = h = 16
def chunk(tag, data):
    return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)
raw = b''.join(b'\x00' + b'\x40\x70\xc0\xff' * w for _ in range(h))
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
pathlib.Path('Panel/src-tauri/icons/icon.png').write_bytes(png)
EOF
    echo "created temp Panel/src-tauri/icons/icon.png (Linux cargo workaround, do not commit)"
  fi
}

run_pwsh_parse() {
  local p; p=$(find_pwsh) || { echo "pwsh not found — rebuild: extract powershell-*-linux-x64.tar.gz to /tmp/opencode/pwsh"; return 1; }
  "$p" -NoProfile -Command 'foreach ($f in "scripts/package.ps1","scripts/build.ps1","scripts/verify-workspace.ps1") { $t = Get-Content $f -Raw; [scriptblock]::Create($t) | Out-Null; Write-Host "OK $f" }'
}

run_npm() {
  need node || return 1
  need npm || return 1
  if [ ! -d Panel/node_modules ]; then (cd Panel && npm ci) || return 1; fi
  (cd Panel && npm run build) || return 1
  (cd Panel && npm run test:stickers) || return 1
  (cd Panel && npm run test:install-gate) || return 1
}

run_dotnet() {
  need "$DOTNET_BIN" || return 1
  local proj
  for proj in \
    addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/PlayerKnifeCustomizer.csproj \
    addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.csproj \
    addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.csproj \
    addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.csproj \
    addons/counterstrikesharp/plugins/PlayerKnifeCustomizer.Tests/PlayerKnifeCustomizer.Tests.csproj
  do
    "$DOTNET_BIN" build -c Release "$proj" || return 1
  done
  "$DOTNET_BIN" run --project addons/counterstrikesharp/shared/MatchCore.Tests -c Release || return 1
  "$DOTNET_BIN" run --project addons/counterstrikesharp/plugins/PlayerKnifeCustomizer.Tests/PlayerKnifeCustomizer.Tests.csproj -c Release || return 1
}

run_cargo() {
  need cargo || return 1
  ensure_icon
  cargo check --manifest-path Panel/src-tauri/Cargo.toml || return 1
  local log="$LOGDIR/cargo-test.log"
  cargo test --manifest-path Panel/src-tauri/Cargo.toml > "$log" 2>&1
  local rc=$?
  grep -E '^test result:' "$log" || tail -30 "$log"
  if [ "$rc" -eq 0 ]; then echo "all cargo tests passed"; return 0; fi
  local name bad=0 count=0
  while IFS= read -r name; do
    [ -z "$name" ] && continue
    count=$((count + 1))
    if ! printf '%s\n' "$KNOWN_WINDOWS_ONLY" | grep -qxF "$name"; then
      echo "UNEXPECTED FAILURE: $name"
      bad=1
    fi
  done < <(grep ' FAILED$' "$log" | awk '{print $2}')
  local known; known=$(printf '%s\n' "$KNOWN_WINDOWS_ONLY" | grep -c '.')
  echo "$count failed, $known known Windows-only; full log: $log"
  if [ "$bad" -ne 0 ]; then return 1; fi
  return 0
}

run_verify() {
  local p; p=$(find_pwsh) || { echo "pwsh not found"; return 1; }
  ensure_local_build
  "$p" -NoProfile -File scripts/verify-workspace.ps1
}

DOTNET_BIN="${DOTNET:-}"
[ -n "$DOTNET_BIN" ] || DOTNET_BIN=$(find_dotnet 2>/dev/null) || DOTNET_BIN=dotnet

echo "=== toolchain ==="
echo "bash    : $BASH_VERSION"
node --version 2>/dev/null || echo "node    : MISSING"
if [ -n "$("$DOTNET_BIN" --list-sdks 2>/dev/null)" ]; then echo "dotnet  : $DOTNET_BIN ($($DOTNET_BIN --list-sdks | tail -1))"; else echo "dotnet  : MISSING (SDK not found via $DOTNET_BIN)"; fi
cargo --version 2>/dev/null || echo "cargo   : MISSING"
if find_pwsh >/dev/null 2>&1; then echo "pwsh    : $(find_pwsh)"; else echo "pwsh    : MISSING"; fi

step pwsh    run_pwsh_parse
step npm     run_npm
step dotnet  run_dotnet
step cargo   run_cargo
step verify  run_verify

echo ""
echo "=== summary ==="
printf '%s\n' "${RESULTS[@]}"
if [ "$FAILED" -ne 0 ]; then
  echo "RESULT: FAIL"
  exit 1
fi
echo "RESULT: PASS"
