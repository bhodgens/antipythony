# T2 Next Steps — 2026-10-27

State: T2-b agent (deleg_8a29e236) died to a provider outage after 37h.
Its uncommitted work (210 lines: pluggy dir() surface, plugin __qualname__
rewrite, int(ExitCode)) was verified and absorbed into commit f2123fd.
Follow-up fixes in 63a80f7.

## Verified working now (commit 63a80f7)

- pytest.main: parses args, resolves rootdir, registers all 30 default
  plugins, reaches collection, reports the session header and exit code.
- pathlib: Path.glob / iterdir / os.scandir iteration all work.
- set/frozenset membership for user-__eq__ members (Path objects).
- SystemExit.code (int conversion for pytest exit codes).
- genexp/map/list lazy drains; scope_of memoized without the ABA bug.
- Gates: cargo test --workspace 412 passed / 0 failed / 1 ignored;
  lxml_differential 24/24.

## The ONE remaining deterministic blocker (next wave, ~1-2h)

_staticmethod unwrapping on metaclass-built classes._

Repro: `vendored_stdlib/enum.py` _simple_enum decorates a class; inside
convert_class, `etype._generate_next_value_` (a staticmethod stored
through the EnumType metaclass path) reads back RAW from
class_attr_get — descriptor_kind() does not classify it, so the call
site dies with `TypeError: 'object' object is not callable`.

Probe: /tmp/t2nonlocal3.py (expects `ok <Color.RED: 1>` on CPython).
Plain-class staticmethod access works (/tmp/t2sm2.py). The failing
lookup is specific to classes built via the metaclass kwargs path
(type(..., boundary=..., _simple=True) → EnumType.__new__ re-stores
dict entries; see vendored_stdlib/enum.py:514-515 where
`staticmethod(_gnv)` is called explicitly).

Fix shape: find how EnumType.__new__ re-stores the class dict (probably
through a different attr storage than the plain class literal) and make
descriptor_kind/class_attr_get classify staticmethod objects there —
or normalize the storage to ObjKind::Descriptor with
descr_kind="staticmethod" at creation.

After that lands: rerun /tmp/t2runner3.py. Expected remaining layer(s):
whatever pytest does next after collection matches (item execution,
assertion rewriting, terminal summary). Budget 1-3 layers by T1
experience (~1 layer/agent-hour, faster now that the hang and the
genexp drains are gone).

## Queued (unchanged)

- issue #10: GC 3-case regression fix wave.
- threading suite: 1 timeout case.
- A-2 adaptations inventory items 4-10 (RustPython donor backlog).
- faulthandler.dump_traceback_later is a no-op (separate issue; write up).
- inspect.getsource broken (tokenize) — separate issue.
- /tmp/lxml-ref-venv gets wiped by /tmp cleaners and loses pyvenv.cfg;
  recreate with `uv venv /tmp/lxml-ref-venv --python 3.14` +
  `uv pip install -p /tmp/lxml-ref-venv lxml` before lxml gates.
