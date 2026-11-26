"""Resolve the workspace build directory for test binaries.

One seam for the .cargo/config.toml target-dir redirect (this workspace
builds into target-0304c, not the cargo-default target/): cargo does NOT
export the redirect to child processes, so suites that path-math into
`REPO/target/<profile>/<bin>` must go through build_dir() instead.

Usage::

    import sys
    from pathlib import Path
    sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
    from _target_dir import build_dir

    cand = build_dir(REPO_ROOT) / "debug" / "ap-parse-check"

Resolution order: $CARGO_TARGET_DIR (exact), else .cargo/config.toml's
target-dir value, else root/target (the cargo default).
"""

from __future__ import annotations

from pathlib import Path


def build_dir(repo_root: Path) -> Path:
    import os

    env = os.environ.get("CARGO_TARGET_DIR", "").strip()
    if env:
        return Path(env)

    config = Path(repo_root) / ".cargo" / "config.toml"
    try:
        text = config.read_text(encoding="utf-8")
    except OSError:
        return Path(repo_root) / "target"

    for line in text.splitlines():
        t = line.strip()
        if t.startswith("target-dir"):
            rest = t[len("target-dir"):].strip()
            if rest.startswith("="):
                rest = rest[1:].strip()
            val = rest.strip().strip('"').strip("'")
            if val:
                return Path(repo_root) / val

    return Path(repo_root) / "target"
