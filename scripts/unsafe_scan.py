#!/usr/bin/env python3
"""Find undocumented `unsafe {` blocks in ap-run/ap-parse (issue #22).
Port of .hermes/audits/unsafe_scan.py scoped to two crates.
Usage: python3 scripts/unsafe_scan.py [--json]
"""
import re, json, pathlib, sys, collections

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGETS = ['crates/ap-run/src', 'crates/ap-parse/src']
pat = re.compile(r'\bunsafe\s*\{')

hits = collections.Counter(); details = []
for d in TARGETS:
    for p in sorted((ROOT / d).rglob('*.rs')):
        lines = p.read_text(errors='replace').splitlines()
        for i, l in enumerate(lines):
            if pat.search(l) and 'forbid' not in l:
                found = False
                j, seen = i - 1, 0
                while j >= 0 and seen < 4:
                    s = lines[j].strip()
                    if s:
                        seen += 1
                        if 'SAFETY' in s.upper():
                            found = True; break
                        if not s.startswith('//'):
                            break
                    j -= 1
                if not found:
                    hits[str(p.relative_to(ROOT))] += 1
                    details.append({"file": str(p.relative_to(ROOT)),
                                    "line": i + 1,
                                    "text": l.strip()[:120]})
if '--json' in sys.argv:
    print(json.dumps({"total": sum(hits.values()), "per_file": dict(hits),
                      "details": details}, indent=1))
else:
    print("UNDOCUMENTED:", dict(hits), "TOTAL", sum(hits.values()))
    for x in details:
        print(f"{x['file']}:{x['line']}  {x['text']}")
