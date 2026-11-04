# Generate expected/<stem>.{out,err,rc} + expected/<stem>.meta for every gc
# corpus case, only if 3 consecutive runs of /opt/homebrew/bin/python3.14
# produce byte-identical stdout+stderr+rc. Refuses to write if a case flaps.
# .meta records provenance (robustness-ci 03.01, finding #11): the reference
# interpreter's actual sys.version, recording date, and this script's sha256.
import hashlib, json, os, subprocess, sys
from datetime import date
from pathlib import Path

GC = Path("/Users/caimlas/git/antipythony/tests/corpus/gc")
PY = "/opt/homebrew/bin/python3.14"
env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}

failed = []
for script in sorted(GC.glob("gc*.py")):
    runs = []
    for _ in range(3):
        p = subprocess.run([PY, str(script)], capture_output=True, env=env,
                           cwd="/Users/caimlas/git/antipythony", timeout=120)
        runs.append((p.stdout, p.stderr, p.returncode))
    if not (runs[0] == runs[1] == runs[2]):
        failed.append(script.name)
        print(f"FLAP {script.name}: runs differ")
        continue
    out, err, rc = runs[0]
    exp = GC / "expected"
    exp.mkdir(exist_ok=True)
    (exp / (script.stem + ".out")).write_bytes(out)
    (exp / (script.stem + ".err")).write_bytes(err)
    (exp / (script.stem + ".rc")).write_text(str(rc) + "\n")
    # Provenance sidecar, one per case, covering the whole triple (03.01).
    sys_version = subprocess.run(
        [PY, "-c", "import sys; print(sys.version)"],
        capture_output=True, text=True, env=env, timeout=60).stdout.strip()
    meta = {
        "sys_version": sys_version,
        "date": date.today().isoformat(),
        "generator_sha256": hashlib.sha256(Path(__file__).resolve().read_bytes()).hexdigest(),
        "family": "gc",
        "generator": "tests/t07_gc/_gen_expected.py",
    }
    (exp / (script.stem + ".meta")).write_text(
        json.dumps(meta, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"PIN  {script.name} rc={rc} out={len(out)}B err={len(err)}B (+meta)")

print("FLAKY:", failed if failed else "none")
sys.exit(1 if failed else 0)
