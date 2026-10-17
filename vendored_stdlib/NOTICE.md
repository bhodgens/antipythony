# Vendored CPython standard-library subset (pure-Python files).

Source: CPython 3.14.7 `Lib/`, copied VERBATIM — original headers kept,
nothing added inside the files.

License: Python Software Foundation License Version 2 (PSF-2.0) — see
`LICENSES/PSF-2.0.txt` at the repo root.

## Resolution order vs native modules (leaf 02.04a)

The native registry (02.08-lite) WINS over this tree: `import json` resolves
natively; `import pathlib` falls through to the file finder and loads from
here. Native modules with registry entries that therefore SHADOW vendored
copies (none vendored, listed for reference): `collections`, `json`, `io`,
`operator`, `functools`, `textwrap`, `string`, `atexit`, `math`, `itertools`,
`heapq`, `bisect`, `os`, `errno`, `platform`, `time`, `linecache`-native,
`random`, `hashlib`, `base64`, `binascii`, `tempfile`, `keyword`.

NOTE: `posixpath.py`/`genericpath.py`/`stat.py`/`linecache.py`/`keyword.py`
ARE vendored even though native registry entries exist — the native registry
is consulted first, so the vendored copies are only reachable if the native
entry is removed. Vendoring them keeps `re`/`glob`/`fnmatch`/`traceback`
import chains (which `import os.path` etc. internally) self-consistent.

## Missing from the local CPython install probe set (not vendored)

- `string.py` — native registry entry exists (02.08-lite); shadowed anyway.
- `atexit.py` — native registry entry exists; shadowed anyway.
- `sysconfig.py` — not in the 37/41 vendor target list; optional for v1.
