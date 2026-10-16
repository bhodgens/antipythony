# Generate expected/<stem>.{out,err,rc} for every gc corpus case,
# only if 3 consecutive runs of /opt/homebrew/bin/python3.14 produce
# byte-identical stdout+stderr+rc. Refuses to write if a case flaps.
import os, subprocess, sys
from pathlib import Path

GC = Path("/Users/caimlas/git/antipythany/tests/corpus/gc")
PY = "/opt/homebrew/bin/python3.14"
env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}

failed = []
for script in sorted(GC.glob("gc*.py")):
    runs = []
    for _ in range(3):
        p = subprocess.run([PY, str(script)], capture_output=True, env=env,
                           cwd="/Users/caimlas/git/antipythany", timeout=120)
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
    print(f"PIN  {script.name} rc={rc} out={len(out)}B err={len(err)}B")

print("FLAKY:", failed if failed else "none")
sys.exit(1 if failed else 0)
