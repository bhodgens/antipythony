# Bughunt wave 6 - 2026-10-22 (commits 487e04a..01260d6, 4 new)

Scope: commit a14fe0d (T1 feat: live-sys.path importer fix + ~30 vendored
stdlib modules, 156 files / 91K lines, scrapy chain "reaches lxml") + 3
doc/plan chores. Method: 2 auditors dispatched; both died on provider
connection errors (23rd/24th consecutive subagent failure), both scopes
executed parent-direct in worktree /tmp/ap-bughunt6. Findings in
/tmp/ap-audit-w6-{runtime,vendored}/findings.json.

This wave RECORDS rather than fixes: every finding lives in the sibling's
ACTIVE T1-leaf zone (its working tree right now has live WIP in
parsel/selector.py, new untracked lxml.rs + jmespath/, and dirty
builtins/importer/interp). Editing those files mid-flight is the
lost-update race the no-stash rule exists for. The findings below are
handed to the T1 leaf owner with exact repro + root cause.

## FINDINGS (not fixed - contested zone)

### HIGH - license compliance: 3 third-party PyPI packages, ZERO attribution
a14fe0d vendored parsel (BSD-3-Clause, Scrapy developers), charset_normalizer
(MIT, Ahmed TAHRI), defusedxml (PSF-contributor, Christian Heimes) into
vendored_stdlib/. Verified NOT CPython stdlib (absent from installed 3.14.7
Lib/). NO attribution anywhere: grep of LICENSES/, THIRD_PARTY_NOTICES,
docs/attribution.md finds none; no LICENSE file inside any of the 3 dirs.
BSD-3 and MIT both require the copyright notice + license text to ship with
redistributions. Violates master.md attribution discipline + the project's
own LICENSES/README policy. defusedxml is PSF-compatible but still needs its
notice preserved.
FIX SHAPE: vendor the 3 license texts (LICENSES/), add THIRD_PARTY_NOTICES
entries, preserve per-file copyright headers; ideally move non-CPython
packages to vendored_thirdparty/ so the CPython-provenance model stays clean.

### HIGH - stdlib-provenance CI gate RED on committed main (191 problems)
`cargo run -p xtask -- stdlib-provenance --check` -> FAIL (237 files, 10
deviated declared, 191 problems). a14fe0d added ~150 files but did NOT touch
DEVIATIONS.toml. Undeclared families: email(30), _pyrepl(26), xml(22),
charset_normalizer(15), defusedxml(11), urllib(6), http(5), pydoc_data(4),
xmlrpc(3), parsel(3), logging(3) + 37 top-level. ci.yml fast-lints runs this
gate -> main's CI is red. The 10 previously-declared deviated files are
intact (no stale-sha regression); all 191 are UNDECLARED-new-file coverage
errors.

### MEDIUM - provenance gate structurally cannot classify third-party packages
The gate model is pristine(==CPython) | deviated(fork-of-CPython, sha-pinned).
parsel/defusedxml/charset_normalizer/jmespath have NO CPython upstream, so
neither status fits. Needs a third status ('third-party' + license + upstream
url + sha) or the vendored_thirdparty/ separation above. Design decision for
the vendoring owner.

### HIGH - import decimal broken: getcontext() returns None
decimal.Decimal('1.1')+Decimal('2.2') -> AttributeError: 'NoneType' object
has no attribute 'rounding'. decimal.getcontext() returns None (CPython:
a Context). The thread-local default-context primitive _pydecimal.py needs
is not provided. ALSO 'Formatter' NameError when decimal is imported
alongside many modules (module-global leak/ordering). Vendored decimal.py
DIFFERS from CPython (ships full _pydecimal content, not the thin shim).
Repro: /tmp/w6_dec2.py, /tmp/w6_many.py.

### MEDIUM - datetime.date.today() raises TypeError
datetime.date.today().year -> TypeError: 'float' object cannot be interpreted
as an integer. _pydatetime.py is byte-IDENTICAL to CPython, so runtime-side:
today()->time.time() (float) needs int coercion at the fromtimestamp boundary
the impl lacks. Other datetime surfaces (construction, isoformat, tz
utcoffset) work. Repro: /tmp/w6_dt.py.

### MEDIUM - intermittent stack overflow on heavy combined imports
A probe importing datetime+decimal+11 more aborted 'thread has overflowed
its stack' (rc=134) on the timedelta line; same modules fine individually.
Related to the known recursion/stack-depth item. Not deterministically
reproduced. Repro: /tmp/w6_rt2.py.

### LOW - _pyrepl/mypy.ini vendored (dev-tool config in runtime tree)
Non-.py mypy config shipped inside vendored_stdlib/; not needed at runtime;
the provenance gate hashes it as a file. Exclude non-.py from the runtime tree.

## CLEAN (byte-exact vs CPython 3.14.7)

datetime.date/timedelta/datetime construction + isoformat + tz.utcoffset;
urllib.parse (quote/urlparse); ipaddress.ip_address; shlex.split; logging
getLogger; email.message Message.get_content_type; xml.etree.ElementTree
fromstring/find/text; numbers.Integral isinstance; selectors.DefaultSelector;
platform.python_version. _pydatetime.py + datetime.py byte-identical to
CPython. collections.py + dataclasses.py modifications declared + shim-only.

## Note on the corpus/gate baseline

pytest at the wave-4 merge was 1912 passed / 0 failed. a14fe0d adds vendored
modules but no new corpus cases yet (T1 is en-route to lxml, not gated). The
provenance-gate red is the one CI-visible regression this commit introduced;
it does not affect cargo test / pytest (those don't run stdlib-provenance),
only the ci.yml fast-lints job.

## Merge note

No fixes to merge this wave (all findings in the sibling's contested T1
zone). This report + the two findings.json files are the deliverable. When
the sibling's T1 leaf lands, re-audit: the license attribution + provenance
declarations + decimal/datetime primitives are the acceptance bar.

## ADDENDUM - full purity sweep (post-report, closes the auditor's >=10-file task)

Diffed EVERY committed vendored_stdlib .py against installed CPython 3.14.7
(not just a 10-file sample). Result:
- CPython-origin files: 49 byte-IDENTICAL, 8 differ, 4 have no upstream.
- Of the 8 that differ, 5 are already DECLARED deviated (_collections_abc,
  _py_warnings, annotationlib, enum, inspect). The 5 UNDECLARED deviations:
  _pydecimal.py, decimal.py, socket.py, pyexpat.py, select.py.
- socket.py/pyexpat.py/select.py are honest self-declared shims (each opens
  with an "antipythany deviation shim" comment citing C-07: they stand in
  for C-extension modules; file://-only T1 gate never does real I/O) - they
  just lack DEVIATIONS.toml entries. decimal.py is a deviation (ships full
  _pydecimal content, not CPython's thin shim). _pydecimal.py rewrites the
  VERBOSE numeric-parser regex to single-line.
- The ~150 "undeclared" the raw gate counts are mostly the 49 pristine files
  (auto-fixable by `stdlib-provenance --fix`) + the third-party dirs, NOT 150
  semantic deviations. The real hand-work is: 5 undeclared deviation entries +
  ~150 pristine `--fix` entries + the third-party attribution (findings 1-3).

NEW ACTIONABLE FINDING (supersedes part of the _pydecimal.py deviation):
- _pydecimal.py's rewrite comment says CPython's VERBOSE multi-line parser
  regex "trips this runtime's sre parser (whitespace/group interaction)".
  That justification is now OBSOLETE: the wave-5 VERBOSE fix (2a6e907,
  skip_verbose before the quantifier token) resolved exactly that class.
  VERIFIED this wave: the ORIGINAL CPython VERBOSE _parser regex (the
  multi-line one _pydecimal.py replaced) now runs byte-exact under
  ap-run-eval on an 8-case battery (signs, int/frac/exp groups, Infinity,
  sNaN+diag, nan, +.5, 1., non-match). So _pydecimal.py can be reverted to
  pristine CPython, eliminating one deviation entirely. Recommend the T1
  owner revert it rather than declare it.

