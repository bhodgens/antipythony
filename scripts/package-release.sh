#!/usr/bin/env bash
# package-release.sh <target-triple> <release-bin-dir> <out-dir>
#
# Leaf 04.03: assemble one release artifact. Used by BOTH
# .github/workflows/release.yml and local release runs (single source of
# truth for packaging — CI is not a special case).
#
# Output: <out-dir>/antipythony-<target>.tar.gz + .tar.gz.sha256 containing
#   bin/antipythony, bin/ap-run-eval
#   vendored_stdlib/          (the runtime stdlib tree; importer resolves it
#                              via ANTIPTYHONY_STDLIB — see importer.rs:177)
#   THIRD_PARTY_NOTICES
#   SHA256SUMS                (in-archive checksums)
set -euo pipefail

TARGET="${1:?usage: package-release.sh <target> <release-bin-dir> <out-dir>}"
BIN_DIR="${2:?}"
OUT_DIR="${3:?}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"   # repo root (script lives in scripts/)

NAME="antipythony-${TARGET}"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/pkg-release.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/$NAME/bin"

# --- binaries ----------------------------------------------------------------
for b in antipythony ap-run-eval; do
    [ -f "$BIN_DIR/$b" ] || { echo "package-release: missing $BIN_DIR/$b" >&2; exit 1; }
    cp "$BIN_DIR/$b" "$STAGE/$NAME/bin/$b"
done

# --- vendored stdlib (runtime requirement, 502 files ~9MB) -------------------
[ -d "$ROOT/vendored_stdlib" ] || { echo "package-release: missing vendored_stdlib/" >&2; exit 1; }
cp -R "$ROOT/vendored_stdlib" "$STAGE/$NAME/vendored_stdlib"

# --- third-party notices -----------------------------------------------------
[ -f "$ROOT/THIRD_PARTY_NOTICES" ] || { echo "package-release: missing THIRD_PARTY_NOTICES" >&2; exit 1; }
cp "$ROOT/THIRD_PARTY_NOTICES" "$STAGE/$NAME/THIRD_PARTY_NOTICES"

# --- aot crt staticlib (`antipythony build` links against it) ------------------
# The build verb locates libap_aot_crt.a relative to the RUNNING BINARY's
# target dir or the baked-in checkout manifest dir (ap-cli/src/build.rs
# crt_staticlib). A release binary on a fresh machine has NEITHER, so the
# archive ships the staticlib and install.sh places it where the error
# message's recovery path can find it via CARGO_TARGET_DIR; the honest
# 04.03 state is recorded in NEXT-0403.md item D3 (making the installed
# binary self-sufficient needs a build.rs lookup extension — ap-cli is
# another agent's surface this leaf).
CRT_CANDIDATES=("$(cd "$ROOT" && ls target*/release/libap_aot_crt.a 2>/dev/null | head -1)" \
                "target/release/libap_aot_crt.a")
CRT=""
for c in "${CRT_CANDIDATES[@]}"; do
    [ -n "$c" ] && [ -f "$ROOT/$c" ] && { CRT="$ROOT/$c"; break; }
done
if [ -n "$CRT" ]; then
    cp "$CRT" "$STAGE/$NAME/libap_aot_crt.a"
else
    echo "package-release: WARNING libap_aot_crt.a not found — archive will not support 'build' off-checkout" >&2
fi

# --- a README so the archive is self-describing ------------------------------
cat > "$STAGE/$NAME/README.txt" <<EOF
antipythony ${TARGET}
runtime version: $(grep -m1 '^version' "$ROOT/Cargo.toml" | awk '{print $3}')

Layout:
  bin/antipythony          the driver (new/add/run/build/cache verbs)
  bin/ap-run-eval          single-expression eval helper (differential testing)
  vendored_stdlib/         the Python 3.14 stdlib tree served by the importer.
                           install.sh places this and sets ANTIPTYHONY_STDLIB;
                           without it, imports of os and friends cannot resolve.
  THIRD_PARTY_NOTICES      full third-party dependency notices (Apache-2.0
                           WITH LLVM-exception workspace; see also LICENSE)

Quick start:
  ./bin/antipythony new myapp && cd myapp
  ../bin/antipythony add rich
  ../bin/antipythony run myapp.py
  ../bin/antipythony build myapp.py -o myapp
EOF

# --- in-archive checksums ----------------------------------------------------
# (sums written via a temp file so the redirect never races the enumeration)
( cd "$STAGE/$NAME" && find . -type f ! -name 'SHA256SUMS*' -print0 \
    | sort -z | xargs -0 sha256sum > SHA256SUMS.tmp && mv SHA256SUMS.tmp SHA256SUMS )

# --- pack + outer checksum ---------------------------------------------------
mkdir -p "$OUT_DIR"
tar -czf "$OUT_DIR/$NAME.tar.gz" -C "$STAGE" "$NAME"
# BSD sha256sum (macOS) vs GNU: use shasum -a 256 as portable fallback.
if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$OUT_DIR" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256" )
else
    ( cd "$OUT_DIR" && shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256" )
fi

echo "packaged $OUT_DIR/$NAME.tar.gz"
