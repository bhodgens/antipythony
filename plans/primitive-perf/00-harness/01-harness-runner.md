# 01-harness-runner — benchmark harness

## DISPATCH INSTRUCTION

Any agent: implement this leaf in `/Users/caimlas/git/antipythony`
(worktree if assigned one). Do NOT commit; write files, run, report.

Parent: plans/primitive-perf/00-harness/orchestrator.md
Estimated context: ~40K. One language (Python). 2 new files + probe set.

## Tasks

### T1 — probe files

Create `tests/bench/primitive/probes/`. One self-contained python file
per primitive family; each file, when run by ANY python (CPython 3.14 or
antipythony), prints machine-readable results to stdout, one JSON object
per line:

```
{"probe": "base64_encode", "size": 49152, "best_s": 0.00006, "gbs": 0.79}
```

Rules:
- Time with `time.perf_counter()`; best-of-5 with 1 warmup iteration;
  record MIN.
- Workload: operate on a 48 KiB bytes buffer AND a 1 MiB buffer; emit
  one line per size. Build buffers deterministically
  (`bytes(range(256)) * k`) so both interpreters do identical work.
- Probe set (initial):
  - `p01_base64.py` — b64encode, b64decode, binascii.b2a_base64,
    binascii.a2b_base64
  - `p03_hexlify.py` — binascii.hexlify, unhexlify
  - `p04_compress.py` — zlib.compress/decompress (level 1 and 6)
  - `p05_hash.py` — hashlib.sha256, md5 (1 MiB)
  - `p06_str.py` — str.find, str.replace, str.split, "\x00".join over a
    large list, str.upper
  - `p07_bytes_int.py` — int.from_bytes/to_bytes, int("big decimal str"),
    hex(large int)
  - `p08_sort.py` — sorted() over 1M random ints (seeded), list.sort
  - `p09_dict.py` — build 1M-entry dict, 1M lookups
  - `p10_json.py` — json.dumps/loads of a nested 1 MiB structure
  Keep each probe under ~80 lines. No f-strings with tabs. Guards:
  skip a workload with a printed note if the interpreter lacks the
  feature (e.g. error message capture must match CPython exactly —
  wrap in try/except and report exception TEXT in the line too).

### T2 — runner

Create `tests/bench/primitive/run_bench.py`:
- Args: `--antipythony <path-to-antipythony-bin>`, `--python <python3.14>`,
  `--out bench_report.json`, `--sizes 48KiB,1MiB` (defaults sane),
  `--only P01,P03` optional filter.
- For each probe: run under CPython (`python3.14 probe.py`) and under
  antipythony (`<bin> run probe.py`), parse the JSON lines, build rows
  per the frozen schema in `00-harness/orchestrator.md`, compute
  ratios, set verdicts (`PASS` if antipythony_gbs >= target_gbs else
  `FAIL`), and write `bench_report.json`.
- Timeout per probe run: 120 s. If antipythony errors on a probe, record
  the row with `verdict: "ERROR"` and the stderr tail — do not crash the
  runner.
- Print a human summary table at the end.

### T3 — self-verify

- Run the runner on the smoke slice (`--only P01,P03`) against
  python3.14 and a built antipythony release binary. Confirm: JSON
  schema-valid, ratios sane, verdicts set.
- Report: file list + the smoke-slice table.

## Self-verification checklist

- [ ] run_bench.py runs green on smoke slice, both interpreters
- [ ] bench_report.json matches the frozen schema exactly
- [ ] probes have no interpreter-conditional code paths (same work both sides)
- [ ] no f-strings containing tabs; each probe < 80 lines

## Review checklist (orchestrator)

- [ ] same-workload guarantee (deterministic buffers, identical loops)
- [ ] min-of-5 timing, warmup present
- [ ] schema matches 00-harness orchestrator contract
- [ ] ERROR path never crashes the runner
