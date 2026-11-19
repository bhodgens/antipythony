#!/usr/bin/env bash
# install.sh — antipythony release installer (leaf 04.03).
#
# Usage:
#   curl -fsSL .../install.sh | bash                  # install latest
#   ./install.sh [--version vX.Y.Z] [--prefix DIR] [--target TRIPLE]
#
# Detects OS/arch -> maps to a release target triple, downloads
# antipythony-<target>.tar.gz + its .sha256 from GitHub releases,
# VERIFIES THE CHECKSUM, then installs to --prefix (default ~/.local/bin).
#
# Env overrides: AP_RELEASE_REPO (default bhodgens/antipythony),
#                AP_RELEASE_VERSION (same as --version),
#                AP_RELEASE_BASE_URL (override the resolved download root;
#                file:// works — this is how the smoke test exercises the
#                REAL installer against a local release mirror, and how
#                air-gapped installs point at an internal mirror),
#                AP_NO_UV=1 (skip the uv bootstrap; --no-uv does the same).
#
# uv bootstrap: `antipythony add` shells out to uv (ap-pack/src/uv.rs —
# subprocess-only policy, never vendored). A fresh machine has neither
# antipythony nor uv, so install.sh bootstraps uv via the official astral
# installer (UV_VERSION pins it; default: latest) unless AP_NO_UV=1 /
# --no-uv. Skipping the bootstrap is exactly what the fresh-machine smoke
# does for its SECOND phase (see scripts/fresh-machine-smoke.sh) — the
# script installs uv itself first so the installer's bootstrap branch is
# exercised on a DIFFERENT run than the no-uv branch.
set -euo pipefail

REPO="${AP_RELEASE_REPO:-bhodgens/antipythony}"
VERSION="${AP_RELEASE_VERSION:-latest}"
PREFIX="${HOME}/.local"
TARGET=""
BASE_URL=""
NO_UV="${AP_NO_UV:-0}"
UV_VERSION="${UV_VERSION:-}"

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?--version requires a value}"; shift 2 ;;
        --prefix)  PREFIX="${2:?--prefix requires a value}";  shift 2 ;;
        --target)  TARGET="${2:?--target requires a value}"; shift 2 ;;
        --no-uv)   NO_UV=1; shift ;;
        -h|--help) usage ;;
        *) echo "install.sh: unknown arg '$1'" >&2; usage ;;
    esac
done

# --- download helper: curl or wget, whichever exists --------------------------
fetch() { # fetch <url> <outfile>
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        echo "install.sh: need curl or wget to download" >&2; exit 1
    fi
}

# --- scratch dir (used by the uv bootstrap download too) ----------------------
TMP="$(mktemp -d "${TMPDIR:-/tmp}/ap-install.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT


# --- uv bootstrap (fresh machines need uv for `add`) ---------------------------
# Skipped when uv is already reachable (upgrade path) or --no-uv/AP_NO_UV=1.
if [ "$NO_UV" != 1 ] && ! command -v uv >/dev/null 2>&1; then
    echo "install.sh: bootstrapping uv${UV_VERSION:+ (pinned ${UV_VERSION})}..."
    if [ -n "$UV_VERSION" ]; then
        # Pinned: download the standalone installer for that version.
        fetch "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-installer.sh" "$TMP/uv-installer.sh"
        UV_UNMANAGED_INSTALL="$PREFIX/bin" sh "$TMP/uv-installer.sh"
    else
        curl -LsSf https://astral.sh/uv/install.sh | sh
    fi
fi

# --- OS/arch -> release target triple -----------------------------------------
if [ -z "$TARGET" ]; then
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    case "$OS" in
        Linux)  gnu=true ;;
        Darwin) gnu=false ;;
        *) echo "install.sh: unsupported OS '$OS' (linux/darwin only)" >&2; exit 1 ;;
    esac
    case "$ARCH" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) echo "install.sh: unsupported arch '$ARCH'" >&2; exit 1 ;;
    esac
    if [ "$gnu" = true ]; then
        # linux: prefer STATIC musl (zero runtime deps), fall back to gnu.
        TARGET="x86_64-unknown-linux-musl"
        [ "$arch" = "aarch64" ] && { \
            echo "install.sh: no linux-aarch64 release target yet (04.03 matrix is x86_64 linux)" >&2; exit 1; }
    else
        case "$arch" in
            x86_64) TARGET="x86_64-apple-darwin" ;;
            aarch64) TARGET="aarch64-apple-darwin" ;;
        esac
    fi
fi

# --- resolve download URLs ----------------------------------------------------
if [ "$VERSION" = "latest" ]; then
    # Follow the latest release redirect to discover the tag.
    LOC="$(fetch "https://github.com/${REPO}/releases/latest" /dev/stdout || true)"
    TAG="$(printf '%s' "$LOC" | grep -o 'tag/[v0-9][^"]*' | head -1 | cut -d/ -f2 || true)"
    [ -n "${TAG:-}" ] || { echo "install.sh: cannot resolve latest release tag" >&2; exit 1; }
else
    TAG="$VERSION"
fi
BASE_URL="${AP_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download/${TAG}}"
NAME="antipythony-${TARGET}"

echo "install.sh: ${NAME} @ ${TAG}"
fetch "${BASE_URL}/${NAME}.tar.gz"       "$TMP/$NAME.tar.gz"
fetch "${BASE_URL}/${NAME}.tar.gz.sha256" "$TMP/$NAME.tar.gz.sha256"

# --- CHECKSUM VERIFY (hard gate; no bypass flag) ------------------------------
EXPECT="$(cut -d' ' -f1 "$TMP/$NAME.tar.gz.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
    GOT="$(sha256sum "$TMP/$NAME.tar.gz" | cut -d' ' -f1)"
else
    GOT="$(shasum -a 256 "$TMP/$NAME.tar.gz" | cut -d' ' -f1)"
fi
if [ "$EXPECT" != "$GOT" ]; then
    echo "install.sh: CHECKSUM MISMATCH for $NAME.tar.gz" >&2
    echo "  expected: $EXPECT" >&2
    echo "  got:      $GOT" >&2
    echo "  refusing to install. (download truncated? release tampered?)" >&2
    exit 1
fi
echo "install.sh: checksum OK ($GOT)"

# --- unpack + install ---------------------------------------------------------
tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
mkdir -p "$PREFIX/bin" "$PREFIX/lib/antipythony"
install -m 0755 "$TMP/$NAME/bin/antipythony"   "$PREFIX/bin/antipythony"
install -m 0755 "$TMP/$NAME/bin/ap-run-eval"   "$PREFIX/bin/ap-run-eval"

# The stdlib tree + notices install to $PREFIX/lib/antipythony so the binary
# can find them without a repo checkout; the importer resolves the tree via
# ANTIPTYHONY_STDLIB, so persist it in the shell profile fragment below.
rm -rf "$PREFIX/lib/antipythony/vendored_stdlib"
cp -R "$TMP/$NAME/vendored_stdlib" "$PREFIX/lib/antipythony/vendored_stdlib"
install -m 0644 "$TMP/$NAME/THIRD_PARTY_NOTICES" "$PREFIX/lib/antipythony/THIRD_PARTY_NOTICES"

# --- aot crt staticlib (installed `build` verb links against it) ---------------
# Installed to $PREFIX/lib/antipythony/. NOTE (04.03 honest state, NEXT-0403
# item D3): ap-cli's crt_staticlib lookup does not yet consult this location,
# so `antipythony build` from an installed binary still needs the checkout's
# target dir (fails loudly naming the fix). Shipping the staticlib now means
# the lookup extension in ap-cli can wire it up without another release change.
if [ -f "$TMP/$NAME/libap_aot_crt.a" ]; then
    install -m 0644 "$TMP/$NAME/libap_aot_crt.a" "$PREFIX/lib/antipythony/libap_aot_crt.a"
    echo "install.sh: crt       -> $PREFIX/lib/antipythony/libap_aot_crt.a"
fi

# --- PATH + ANTIPTYHONY_STDLIB persistence ------------------------------------
STDLIB_DIR="$PREFIX/lib/antipythony/vendored_stdlib"
case ":${SHELL}:" in
    *zsh*) RC_FILE="$HOME/.zshrc" ;;
    *)     RC_FILE="$HOME/.bashrc" ;;
esac
if ! grep -q "ANTIPTYHONY_STDLIB" "$RC_FILE" 2>/dev/null; then
    {
        echo ''
        echo '# added by antipythony install.sh'
        echo "export ANTIPTYHONY_STDLIB=\"$STDLIB_DIR\""
        case ":$PATH:" in
            *":$PREFIX/bin:"*) ;;  # already on PATH
            *) echo "export PATH=\"$PREFIX/bin:\$PATH\"" ;;
        esac
    } >> "$RC_FILE"
    echo "install.sh: updated $RC_FILE (ANTIPTYHONY_STDLIB + PATH)"
fi

echo "install.sh: installed -> $PREFIX/bin/{antipythony,ap-run-eval}"
echo "install.sh: stdlib    -> $STDLIB_DIR"
echo "install.sh: notices   -> $PREFIX/lib/antipythony/THIRD_PARTY_NOTICES"
echo "install.sh: open a new shell (or: source $RC_FILE), then: antipythony new myapp"
