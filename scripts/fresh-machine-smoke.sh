#!/usr/bin/env bash
# fresh-machine-smoke.sh <target-triple> <dist-dir>
#
# THE leaf 04.03 GATE, scripted: install -> new -> add rich -> run -> build
# against a release artifact, the way a brand-new user machine would.
#
# Preconditions (the caller guarantees; release.yml's fresh-machine-smoke job
# asserts them): NO cargo/rustc/uv on PATH, no repo checkout influence, cold
# ANTIPTYHONY_CACHE. scripts/install.sh does the install (checksum-verified);
# everything after runs through the INSTALLED binary from a clean HOME.
set -euo pipefail

TARGET="${1:?usage: fresh-machine-smoke.sh <target> <dist-dir>}"
DIST="${2:?}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

step() { printf '\n=== smoke: %s\n' "$*"; }
fail() { printf '\n=== SMOKE FAIL: %s\n' "$*" >&2; exit 1; }

# --- simulate a fresh machine -------------------------------------------------
# Scratch HOME + scratch project dir + cold cache; scrub PATH of cargo/uv so
# even a leakage from the runner image cannot satisfy the `add` dependency.
FRESH_HOME="$(mktemp -d "${TMPDIR:-/tmp}/ap-fresh.XXXXXX")"
export HOME="$FRESH_HOME"
export ANTIPTYHONY_CACHE="$FRESH_HOME/.cache/antipythony"
export ANTIPTYHONY_UV=""
# The installer needs a version tag; the file:// mirror has no "latest" redirect,
# so pin a stand-in tag (the checksum gate, not the tag, is what matters here).
export AP_RELEASE_VERSION="${AP_RELEASE_VERSION:-v0.0.0-smoke}"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/ap-smoke-proj.XXXXXX")"
PATH_NOCARGO="$(printf '%s' "$PATH" | tr ':' '\n' \
    | grep -v -e '/cargo/' -e '/.cargo/' -e '/rustc/' -e 'astral' \
    | paste -sd: -)"
cd "$SCRATCH"

# install.sh writes to $HOME/.local and $HOME/.bashrc — inside the scratch
# HOME, so the real machine profile is untouched. AP_RELEASE_BASE_URL points
# the REAL installer at a local file:// mirror of the release layout, so the
# checksum-verify path is exercised exactly as in production (no shim).
# NO UV_VERSION is passed here on purpose: the fresh-container case lets
# install.sh run its UNPINNED astral bootstrap (the pinned-branch test runs
# separately in CI); locally, uv may already be on the scrubbed PATH and the
# installer's upgrade path skips it.
mkdir -p "$SCRATCH/relmirror"
cp "$DIST/antipythony-${TARGET}.tar.gz" \
   "$DIST/antipythony-${TARGET}.tar.gz.sha256" "$SCRATCH/relmirror/"
AP_RELEASE_BASE_URL="file://$SCRATCH/relmirror" \
    bash "$REPO_ROOT/scripts/install.sh" --target "$TARGET" --prefix "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH_NOCARGO"
export ANTIPTYHONY_STDLIB="$HOME/.local/lib/antipythony/vendored_stdlib"

command -v antipythony >/dev/null 2>&1 || fail "antipythony not on PATH after install"
step "installed: $(command -v antipythony)"

step "new demoapp"
antipythony new demoapp || fail "new demoapp"
[ -f demoapp/antipythony.toml ] || fail "antipythony.toml missing after new"
[ -f demoapp/demoapp.py ] || fail "demoapp.py missing after new"
cd demoapp

step "add rich (uv fetch, pure-wheel gate)"
antipythony add rich || fail "add rich"
grep -q 'rich' antipythony.toml || fail "rich not recorded in antipythony.toml"

step "run demoapp.py"
OUT="$(antipythony run demoapp.py)" || fail "run demoapp.py (rc=$?)"
printf 'output: %s\n' "$OUT"
printf '%s' "$OUT" | grep -q "hello, antipythony" || fail "run output missing 'hello, antipythony'"

step "build demoapp.py -o demoapp-bin"
antipythony build demoapp.py -o demoapp-bin || fail "build demoapp.py"
[ -x demoapp-bin ] || fail "build output not executable"
# The built binary must itself run (AoT artifact executes standalone).
BUILD_OUT="$(./demoapp-bin)" || fail "built binary execution"
printf 'built-binary output: %s\n' "$BUILD_OUT"

step "SMOKE GREEN (install -> new -> add rich -> run -> build)"
