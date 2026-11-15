#!/usr/bin/env python3
"""Leaf 10.01 measurement: scan real PyPI wheels of top native extensions.

For each package: download the most-recent manylinux x86_64 CPython wheel,
find every shared object inside, dump undefined + defined dynamic symbols
(nm -D), bucket undefined symbols into C-API surface categories.

Read-only research tool (leaf 10.01). Nothing redistributed; test-data use only.
"""
import json
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from collections import Counter
from pathlib import Path

from pip._internal.commands import create_command  # noqa: F401  (verifies pip importable)

# Top PyPI packages with native C/C++/Rust extensions (downstream-dep-weighted,
# matches the master tier table + the Top-5000 most-depended-on list).
PACKAGES = [
    # name, import-name-of-interest (unused), note
    ("numpy", "array compute, tier T1-adjacent"),
    ("pydantic-core", "Rust ext, blocker for T3/T4"),
    ("pydantic", "wrapper (may pull pydantic-core)"),
    ("cryptography", "Rust ext _rust"),
    ("cffi", "FFI"),
    ("regex", "C ext"),
    ("lxml", "C ext"),
    ("pillow", "C ext"),
    ("markupsafe", "C ext"),
    ("greenlet", "C ext"),
    ("msgpack", "C ext"),
    ("orjson", "Rust ext"),
    ("uvloop", "C ext"),
    ("httptools", "C ext"),
    ("rpds-py", "Rust ext"),
    ("pydantic-settings", "pure (control)"),
    ("contourpy", "C ext"),
    ("kiwisolver", "C ext"),
    ("multidict", "C ext"),
    ("frozenlist", "C ext"),
    ("aiohttp", "C ext"),
    ("cryptography-void", None),  # placeholder removed below
]
PACKAGES = [p for p in PACKAGES if p[1] is not None]
PACKAGES.append(("bcrypt", "Rust ext"))
PACKAGES.append(("pyyaml", "C ext"))
PACKAGES.append(("sqlalchemy", "C ext (optional)"))
PACKAGES.append(("tortoise-orjson", None))
PACKAGES.pop()  # remove the None stub
PACKAGES.append(("protobuf", "upb C ext"))
PACKAGES.append(("google-crc32c", "C ext"))
PACKAGES.append(("grpcio", "C ext"))
PACKAGES.append(("pyjwt", "pure (control)"))
PACKAGES.append(("tiktoken", "Rust ext"))
PACKAGES.append(("pyzstd", "C ext"))
PACKAGES.append(("brotli", "C ext"))
PACKAGES.append(("zstandard", "C ext"))

CP = "cp312"  # broad-availability ABI tag
PLATFORM = "manylinux_2_17_x86_64"

def find_pip():
    return [sys.executable, "-m", "pip"]

def download(pkg: str, tmp: Path):
    dest = tmp / "dl" / pkg
    dest.mkdir(parents=True, exist_ok=True)
    cmd = find_pip() + [
        "download", pkg, "--no-deps", "--only-binary", ":all:",
        "--platform", PLATFORM, "--python-version", "3.12",
        "--implementation", "cp", "--abi", f"{CP}", "-d", str(dest),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    if r.returncode != 0:
        return None, r.stderr.strip().splitlines()[-1:] 
    wheels = sorted(dest.glob("*.whl"))
    return (wheels[-1] if wheels else None), None

def scan_wheel(wheel: Path, tmp: Path):
    """Return (per-so undefined symbols, defined sym count, so list)."""
    ex = tmp / wheel.stem
    ex.mkdir(exist_ok=True)
    if wheel.name.endswith(".whl"):
        with zipfile.ZipFile(wheel) as z:
            z.extractall(ex)
    else:
        with tarfile.open(wheel) as t:
            t.extractall(ex)

    sos = sorted(ex.rglob("*.so")) + sorted(ex.rglob("*.so.*"))
    all_undef = Counter()
    all_def = Counter()
    so_report = []
    for so in sos:
        try:
            u = subprocess.run(["nm", "-D", "--undefined-only", str(so)],
                               capture_output=True, text=True, timeout=60)
            d = subprocess.run(["nm", "-D", "--defined-only", str(so)],
                               capture_output=True, text=True, timeout=60)
        except subprocess.TimeoutExpired:
            continue
        undef = [l.split()[-1] for l in u.stdout.splitlines() if l.strip()]
        defined = [l.split()[-1] for l in d.stdout.splitlines() if l.strip()]
        # strip @@VERSION suffixes
        undef = [s.split("@")[0] for s in undef]
        defined = [s.split("@")[0] for s in defined]
        all_undef.update(undef)
        all_def.update(defined)
        so_report.append({
            "so": str(so.relative_to(ex)),
            "undefined": len(undef),
            "defined": len(defined),
        })
    return all_undef, all_def, so_report

def classify(symbols):
    """Bucket undefined symbols into C-API surface categories."""
    py_api = sorted({s for s in symbols if s.startswith("Py") or s.startswith("_Py")})
    cats = Counter()
    detail = {}
    def add(cat, syms):
        syms = sorted(set(syms))
        if syms:
            cats[cat] += len(syms)
            detail[cat] = syms
    add("object/protocol", [s for s in py_api if any(
        s.startswith(p) for p in (
            "PyObject_", "PySequence_", "PyMapping_", "PyIter_", "PyNumber_",
            "PyObject_", "PyCallable_", "PyBuffer_", "_PyObject_"))])
    add("refcount", [s for s in py_api if "INCREF" in s or "DECREF" in s
                     or s.startswith("Py_XSETREF") or s.startswith("Py_SETREF")])
    add("type", [s for s in py_api if s.startswith("PyType_")
                 or s in ("PyObject_GetAttr", "PyObject_SetAttr")])
    add("list/tuple/dict/set", [s for s in py_api if any(
        s.startswith(p) for p in ("PyList_", "PyTuple_", "PyDict_", "PySet_",
                                  "PyFrozenSet_", "PyFrozenList"))])
    add("str/unicode/bytes", [s for s in py_api if any(
        s.startswith(p) for p in ("PyUnicode_", "PyBytes_", "PyByteArray_", "PyStr_"))])
    add("int/long/float", [s for s in py_api if any(
        s.startswith(p) for p in ("PyLong_", "PyFloat_", "PyNumber_", "Py_ARITHMETIC"))])
    add("module/init", [s for s in py_api if any(k in s for k in
        ("Module", "ModuleDef", "Init", "modexec", "Module_Create"))])
    add("argument/parse", [s for s in py_api if any(k in s for k in
        ("Arg_Parse", "Arg_ParseTuple", "Arg_ParseTupleAndKeywords", "Arg_Unpack",
         "VaBuild", "PyArg", "GetArg"))])
    add("exception", [s for s in py_api if "Err" in s])
    add("import/sys", [s for s in py_api if any(k in s for k in
        ("Import", "Sys_", "Run_", "RunMain"))])
    add("thread/gil", [s for s in py_api if any(k in s for k in
        ("Thread", "GILState", "GilState", "BeginAllowThreads", "EndAllowThreads"))])
    add("memory/alloc", [s for s in py_api if any(k in s for k in
        ("Mem_", "Alloc", "Free", "Realloc", "MemNew", "Calloc"))])
    add("frame/code/eval", [s for s in py_api if any(k in s for k in
        ("Frame", "Code_", "Eval_", "Compile", "EvalCode"))])
    add("weakref/gc", [s for s in py_api if any(k in s for k in
        ("WeakRef", "Weak", "GC_", "Gc_"))])
    add("capsule/cobject/ctx", [s for s in py_api if any(k in s for k in
        ("Capsule", "CObject", "Context", "Context_New"))])
    add("datetime", [s for s in py_api if "DateTime" in s or "DateTime_" in s])
    add("slice", [s for s in py_api if "Slice" in s or "Slice_" in s])
    add("type-from-spec", [s for s in py_api if "FromSpec" in s or "FromMetaclass" in s])
    add("other-py", [s for s in py_api])  # fill-in, counts total
    return cats, py_api, detail

def main():
    tmp = Path(tempfile.mkdtemp(prefix="ce-survey-"))
    results = {}
    for pkg, note in PACKAGES:
        try:
            wheel, err = download(pkg, tmp)
            if wheel is None:
                results[pkg] = {"error": err or "no wheel", "note": note}
                print(f"[SKIP] {pkg}: {results[pkg]['error']}", flush=True)
                continue
            undef, defined, so_report = scan_wheel(wheel, tmp)
            cats, py_api, detail = classify(undef)
            results[pkg] = {
                "wheel": wheel.name,
                "note": note,
                "so_count": len(so_report),
                "so_report": so_report,
                "undefined_total": sum(undef.values()),
                "undefined_unique": len(undef),
                "py_api_undefined_unique": len(py_api),
                "py_api_undefined": sorted(py_api),
                "categories": dict(cats),
                "defined_total": sum(defined.values()),
            }
            print(f"[OK] {pkg}: {len(so_report)} .so, {len(py_api)} Py* undefined syms", flush=True)
        except Exception as e:
            results[pkg] = {"error": repr(e), "note": note}
            print(f"[ERR] {pkg}: {e}", flush=True)
    out = Path(__file__).parent / "wheel-scan-results.json"
    out.write_text(json.dumps(results, indent=1))
    print(f"wrote {out}")

if __name__ == "__main__":
    main()
