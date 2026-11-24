"""zoneinfo._tzpath shim (F-STRUCT-1 stage).

CPython exposes zoneinfo._tzpath as a C-accelerated submodule; this runtime
has no C zoneinfo module, so serve the pure-py fallback (mirrors the
paths of CPython's zoneinfo/_tzpath.py consumed by zoneinfo/__init__.py).
"""
import os
import pathlib

TZFILES = [
    "posix/",
    "",
]

DEFAULT_TZPATH = [
    "/usr/share/zoneinfo",
    "/usr/lib/zoneinfo",
    "/usr/share/lib/zoneinfo",
    "/etc/zoneinfo",
]

TZPATH = list(DEFAULT_TZPATH)


def reset_tzpath(to=None):
    global TZPATH
    if to is None:
        TZPATH = list(DEFAULT_TZPATH)
        try:
            env_tz = os.environ.get("PYTHONTZPATH")
            if env_tz is not None:
                TZPATH = env_tz.split(":")
        except Exception:
            pass
        return
    if isinstance(to, (str, bytes)):
        to = [to]
    TZPATH = list(to)


def available_timezones():
    out = set()
    for root in TZPATH:
        base = pathlib.Path(root)
        if not base.is_dir():
            continue
        for path in base.rglob("*"):
            if path.is_file() and path.suffix == "" and not path.name.startswith("."):
                rel = path.relative_to(base)
                parts = [p for p in rel.parts if p != "posix"]
                if parts:
                    out.add("/".join(parts))
    return out


class InvalidTZPathWarning(RuntimeWarning):
    pass
