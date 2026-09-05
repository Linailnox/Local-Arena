#!/usr/bin/env bash
#
# auto-preview-release.sh
#
# Automates the "release test CI" flow used by Local Arena maintainers:
#  1. base  = current checked-out branch
#  2. pick  = next free test_build_N name (local + upstream remote)
#  3. branch pick off base, strip CI signing from .github/workflows/release.yml
#  4. commit "tmp: remove signing for test build"
#  5. pick next free v1.4.3.3-Preview.M tag (local + upstream remote)
#  6. tag pick, push branch + tag to the fork remote (default: upstream)
#  7. checkout back to base
#
# Set UPSTREAM_REMOTE to override the fork remote (default "upstream").
#
set -euo pipefail

REPO="$(git rev-parse --show-toplevel)"
cd "$REPO"

UPSTREAM="${UPSTREAM_REMOTE:-upstream}"
BRANCH_PREFIX="test_build_"
TAG_PREFIX="v1.4.3.3-Preview."

die() { echo "error: $*" >&2; exit 1; }

BASE="$(git branch --show-current || true)"
[ -n "$BASE" ] || die "not on a branch (detached HEAD?)"
git config "remote.${UPSTREAM}.url" >/dev/null 2>&1 || die "remote '${UPSTREAM}' is not configured"

# Refuse to run over uncommitted tracked changes (ignored build dirs are fine).
if [ -n "$(git status --porcelain)" ]; then
    die "working tree has uncommitted changes; commit or stash them first"
fi

echo "== base branch: ${BASE} =="
echo "== fork remote: ${UPSTREAM} =="

# --- next free test_build_N -------------------------------------------------
max_branch=0
for b in $(git for-each-ref --format='%(refname:short)' "refs/heads/${BRANCH_PREFIX}*"); do
    n="${b##*${BRANCH_PREFIX}}"
    case "$n" in ''|*[!0-9]*) continue ;; esac
    [ "$n" -gt "$max_branch" ] && max_branch="$n"
done
# names already on the fork (they may not be fetched locally)
while read -r _ ref; do
    [ -n "$ref" ] || continue
    name="${ref#refs/heads/}"
    n="${name##*${BRANCH_PREFIX}}"
    case "$n" in ''|*[!0-9]*) continue ;; esac
    [ "$n" -gt "$max_branch" ] && max_branch="$n"
done < <(git ls-remote --heads "$UPSTREAM" "refs/heads/${BRANCH_PREFIX}*" 2>/dev/null || true)
BRANCH_NAME="${BRANCH_PREFIX}$((max_branch + 1))"

# --- next free Preview tag --------------------------------------------------
max_tag=0
for t in $(git tag -l "${TAG_PREFIX}*"); do
    n="${t##*${TAG_PREFIX}}"
    case "$n" in ''|*[!0-9]*) continue ;; esac
    [ "$n" -gt "$max_tag" ] && max_tag="$n"
done
while read -r _ ref; do
    [ -n "$ref" ] || continue
    [[ "$ref" == *'^{}' ]] && continue
    tag="${ref#refs/tags/}"
    n="${tag##*${TAG_PREFIX}}"
    case "$n" in ''|*[!0-9]*) continue ;; esac
    [ "$n" -gt "$max_tag" ] && max_tag="$n"
done < <(git ls-remote --tags "$UPSTREAM" "${TAG_PREFIX}*" 2>/dev/null || true)
TAG_NAME="${TAG_PREFIX}$((max_tag + 1))"

echo "== branch: ${BRANCH_NAME} =="
echo "== tag:    ${TAG_NAME} =="

git ls-remote --heads "$UPSTREAM" "refs/heads/${BRANCH_NAME}" 2>/dev/null | grep -q . \
    && die "branch ${BRANCH_NAME} already exists on ${UPSTREAM}"
git ls-remote --tags "$UPSTREAM" "refs/tags/${TAG_NAME}" 2>/dev/null | grep -q . \
    && die "tag ${TAG_NAME} already exists on ${UPSTREAM}"

# --- create test branch ------------------------------------------------------
git switch -c "$BRANCH_NAME" "$BASE" >/dev/null
restore_base() { git switch -q "$BASE" >/dev/null 2>&1 || true; }
trap restore_base EXIT

# --- strip signing from the release workflow --------------------------------
WORKFLOW=".github/workflows/release.yml"
[ -f "$WORKFLOW" ] || die "missing ${WORKFLOW}"
python3 - "$WORKFLOW" > "${WORKFLOW}.tmp" <<'PY'
import sys
path = sys.argv[1]
lines = open(path, encoding="utf-8").read().splitlines(keepends=True)

def strip_block(lines):
    out = []
    skipping = False
    for ln in lines:
        if ln.startswith("      - name: Verify signed update assets"):
            skipping = True
            continue
        if skipping:
            if ln.startswith("      - name:"):
                skipping = False
            else:
                continue
        out.append(ln)
    return out

out = []
for ln in strip_block(lines):
    s = ln
    if s.startswith("name: Build signed release"):
        s = s.replace("Build signed release", "Build release (test)")
    elif "CSBIP_UPDATE_SIGNING_KEY" in s:
        continue  # env secret + the two guard lines under "Prepare build..."
    elif s.strip() == '- "v1.4.3.3"':
        continue  # only Preview tags trigger test builds
    elif "Prepare build and signing tools" in s:
        s = s.replace("Prepare build and signing tools", "Prepare build tools")
    elif s.strip().startswith("python -m pip install"):
        continue
    out.append(s)
sys.stdout.write("".join(out))
PY
mv "${WORKFLOW}.tmp" "$WORKFLOW"

# Semantic self-check: only the Preview tag may trigger, and no signing remains.
if grep -q 'CSBIP_UPDATE_SIGNING_KEY\|Verify signed update assets\|pynacl' "$WORKFLOW"; then
    die "signing artifacts remain in ${WORKFLOW}; refusing to push"
fi
if ! grep -q 'v1.4.3.3-Preview.\*' "$WORKFLOW" || grep -q '"v1.4.3.3"' "$WORKFLOW"; then
    die "unexpected tag trigger in ${WORKFLOW}; refusing to push"
fi

git add "$WORKFLOW"
AUTHOR_NAME="$(git config user.name || true)"
AUTHOR_EMAIL="$(git config user.email || true)"
if [ -z "$AUTHOR_NAME" ]; then AUTHOR_NAME="Magichear"; fi
if [ -z "$AUTHOR_EMAIL" ]; then AUTHOR_EMAIL="1596925336@qq.com"; fi
git -c user.name="$AUTHOR_NAME" -c user.email="$AUTHOR_EMAIL" commit --no-verify -q -m "tmp: remove signing for test build"

# --- tag, push, restore branch ----------------------------------------------
git tag "$TAG_NAME"

echo "== pushing branch ${BRANCH_NAME} =="
git push -u "$UPSTREAM" "$BRANCH_NAME"
echo "== pushing tag ${TAG_NAME} =="
git push "$UPSTREAM" "$TAG_NAME"

REPO_URL="$(git config "remote.${UPSTREAM}.url")"
WEB_URL="$(python3 -c "import sys,re;u=sys.argv[1];u=re.sub(r'^(git@[^:]+:|https?://[^/]+/|ssh://[^/]+/|git://[^/]+/)','',u);u=re.sub(r'\.git$','',u);print('https://github.com/'+u)" "$REPO_URL" 2>/dev/null || true)"

# --- report the Actions run for this tag ----------------------------
SLUG="$(python3 -c "import sys,re;u=sys.argv[1];u=re.sub(r'^(git@[^:]+:|https?://[^/]+/|ssh://[^/]+/|git://[^/]+/)','',u);u=re.sub(r'\.git$','',u);print(u)" "$REPO_URL" 2>/dev/null || true)"
TAG_SHA="$(git rev-parse "$TAG_NAME" 2>/dev/null || true)"
RUN_URL=""
if [ -n "$SLUG" ] && [ -n "$TAG_SHA" ]; then
    AUTH=()
    if [ -n "${GH_TOKEN:-}${GITHUB_TOKEN:-}" ]; then
        AUTH=(-H "Authorization: Bearer ${GITHUB_TOKEN:-$GH_TOKEN}")
    fi
    for _ in $(seq 1 20); do
        sleep 8
        BODY="$(curl -fsSL "${AUTH[@]}" "https://api.github.com/repos/${SLUG}/actions/runs?head_sha=${TAG_SHA}&per_page=10" 2>/dev/null || true)"
        RUN_URL="$(python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    for r in d.get('workflow_runs', []):
        if r.get('event') == 'push' and str(r.get('head_sha','')).startswith('${TAG_SHA}'):
            print(r.get('html_url','')); break
except Exception:
    pass
" <<< "$BODY" 2>/dev/null || true)"
        [ -n "$RUN_URL" ] && break
    done
fi

echo
echo "Done."
echo "  branch : ${BRANCH_NAME} -> ${UPSTREAM}"
echo "  tag    : ${TAG_NAME}"
if [ -n "$RUN_URL" ]; then
    echo "  CI run : ${RUN_URL}"
else
    echo "  CI     : watch ${WEB_URL:-https://github.com}/actions (tag ${TAG_NAME})"
fi
