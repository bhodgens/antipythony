# 02-str-and-bytes — audit P06+P07

## DISPATCH INSTRUCTION

Audit-only leaf. Do NOT edit implementation code. Do NOT commit.
Parent: plans/primitive-perf/02-parity-audit/orchestrator.md. ~60K.

## Tasks

### T1 — baseline rows
P06 (str find/replace/split/join/upper) + P07 (int.from_bytes/to_bytes,
int(str), hex) rows in bench_report.json.

### T2 — our implementations
- str methods: ap-obj/src/value.rs + ap-run string method arms
  (methods.rs). Byte-slice ops on UTF-8? per-char loops?
- int(str) decimal: ap-obj/src/int.rs (num-bigint parse? hand-rolled?)
- from_bytes/to_bytes: same file

### T3 — compare
- CPython: str.find → memchr/Crochemore-Perrin two-way; str.replace →
  memchr-accelerated; int(str) → _pylong.py longsize threshold + C
  _decimal (libmpdec); small ints → strtol-class
- Rust SOTA: memchr crate (SIMD, 10x naive), aho-corasick for multi,
  num-bigint parse (already our dep?) or fast decconv (fearless_simd
  decconv techniques)
- Per-primitive: ratio, gap, root cause, verdict

### T4 — findings doc
`audits/str-bytes.md`, same verdict table shape as 01.

## Self-verification checklist
- [ ] every P06/P07 row explained
- [ ] memchr-crate opportunity costed (new dep — flag for master:
      dependency posture decision needed)
- [ ] file:line cites for our impls

## Review checklist (orchestrator)
- [ ] int(str) analysis distinguishes small-int vs bigint paths
- [ ] any new-dep proposal flagged explicitly for the user decision
