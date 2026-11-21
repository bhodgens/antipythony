#!/usr/bin/env python3
"""Leaf 10.01: download top PyPI native-extension wheels and inventory their C-API usage.

For each package: query PyPI JSON API, pick a recent platform wheel
(macosx arm64 preferred, else manylinux x86_64/aarch64), download, unzip,
then for every .so: `nm -gU` -> undefined (imported) and defined (exported,
PyInit_*) symbols. Output: JSON inventory at ce-scan-results.json.
"""
import json, os, subprocess, sys, tempfile, urllib.request, re
from pathlib import Path

PACKAGES = [
    # (pypi name, import/monitoring interest note)
    "pydantic-core",
    "cryptography",
    "numpy",
    "pandas",
    "pyarrow",
    "orjson",
    "pillow",
    "lxml",
    "regex",
    "psycopg2",       # psycopg2-binary actually ships wheels; try both
    "psycopg2-binary",
    "grpcio",
    "pyyaml",
    "markupsafe",
    "greenlet",
    "cffi",
    "pycparser",      # pure python control (expect: no wheel)
    "bcrypt",
    "uvloop",
    "httptools",
    "msgspec",
    "msgpack",
    "rpds-py",
    "tokenizers",
    "safetensors",
    "tiktoken",
    "watchfiles",
    "contourpy",
    "kiwisolver",
    "matplotlib",     # meta, but ships some extension wheels
    "zstandard",
    "brotli",
    "brotlicffi",
    "pyjwt",          # pure python control
    "mysqlclient",
    "pymupdf",
    "rapidfuzz",
    "python-dateutil",  # pure control
    "shapely",
    "fiona",
    "numpy",
    "scipy",
    "pydantic",       # meta package - wheel via pydantic-core
    "sqlalchemy",
    "argon2-cffi",
    "cryptography",
    "gevent",
    "psutil",
    "ujson",
    "simdjson",
    "pymerkle",       # likely pure
]

WHEEL_PAT = re.compile(r"^(cp3\d+|abi3|py3)-cp3\d*|py3-none|cp3\d+-cp3\d+")
GOOD_TAGS = ("macosx_11_0_arm64", "macosx_12_0_arm64", "macosx_13_0_arm64",
             "macosx_14_0_arm64", "macosx_10_12_x86_64", "macosx_10_9_x86_64")

def fetch_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": "ce-survey-scan/0.1"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())

def pick_wheel(releases):
    """Return (url, filename, tagclass) best platform wheel or None."""
    cands = []
    for f in releases:
        fn = f["filename"]
        if not fn.endswith(".whl"):
            continue
        if any(g in fn for g in GOOD_TAGS):
            cands.append(f)
    if not cands:
        return None
    # prefer newest python tag, abi3, arm64 macosx
    def score(f):
        fn = f["filename"]
        s = 0
        if "arm64" in fn: s += 4
        if "abi3" in fn: s += 2
        if "cp314" in fn: s += 8
        elif "cp313" in fn: s += 6
        elif "cp312" in fn: s += 5
        elif "cp311" in fn: s += 4
        return s
    cands.sort(key=score, reverse=True)
    return cands[0]

def scan_so(path):
    """Return (undefined Py symbols, defined Py symbols incl PyInit)."""
    undefined, defined = set(), set()
    try:
        # NOTE: on Mach-O, nm -U *suppresses* undefined syms and C symbols get a
        # leading underscore. Use plain `nm` and strip one leading underscore.
        out = subprocess.run(["nm", str(path)], capture_output=True,
                             text=True, timeout=120)
        for line in out.stdout.splitlines():
            parts = line.split()
            if len(parts) < 3:
                continue
            kind, sym = parts[-2], parts[-1]
            if sym.startswith("_"):
                sym = sym[1:]
            if kind == "U":
                undefined.add(sym)
            else:
                defined.add(sym)
    except Exception as e:
        print(f"  nm failed for {path}: {e}", file=sys.stderr)
    return sorted(undefined), sorted(defined)

def main():
    results = {}
    outdir = Path(os.environ.get("CE_SCAN_DIR", "/tmp/ce-scan"))
    outdir.mkdir(exist_ok=True)
    for pkg in PACKAGES:
        key = f"{pkg}"
        if key in results:
            continue
        print(f"=== {pkg}", flush=True)
        try:
            meta = fetch_json(f"https://pypi.org/pypi/{pkg}/json")
        except Exception as e:
            results[key] = {"error": f"pypi fetch: {e}"}
            continue
        version = meta["info"]["version"]
        wheel = pick_wheel(meta["releases"].get(version, []))
        if not wheel:
            results[key] = {"version": version, "wheel": None,
                            "note": "no platform wheel at latest release"}
            continue
        url = wheel["url"]
        fn = wheel["filename"]
        dest = outdir / fn
        if not dest.exists():
            try:
                urllib.request.urlretrieve(url, dest)
            except Exception as e:
                results[key] = {"version": version, "error": f"download: {e}"}
                continue
        # unzip
        ex = dest.with_suffix(".extracted")
        subprocess.run(["unzip", "-oq", str(dest), "-d", str(ex)], timeout=120)
        so_files = list(ex.rglob("*.so"))
        pkg_undef, pkg_def, init_syms = set(), set(), []
        for so in so_files:
            u, d = scan_so(so)
            pkg_undef |= set(u)
            pkg_def |= set(d)
            init_syms += [s for s in d if s.startswith("_PyInit_") or s.startswith("PyInit_")]
        results[key] = {
            "version": version,
            "wheel": fn,
            "so_count": len(so_files),
            "undefined_Py": sorted(s for s in pkg_undef if s.startswith("Py") or s.startswith("_Py")),
            "undefined_all_count": len(pkg_undef),
            "defined_Py_count": len([s for s in pkg_def if s.startswith("Py") or s.startswith("_Py")]),
            "PyInit": init_syms,
            "uses_abi3": "abi3" in fn or any("abi3" in s for s in pkg_undef),
        }
        print(f"  {fn}: {len(so_files)} .so, {len(results[key]['undefined_Py'])} undefined Py syms", flush=True)
    Path(os.environ.get("CE_SCAN_OUT", "/tmp/ce-scan/results.json")).write_text(
        json.dumps(results, indent=2))
    print("WROTE results")

if __name__ == "__main__":
    main()
