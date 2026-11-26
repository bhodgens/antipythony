# Host-adaptive pins (GitHub issue #17)

Status: mechanism landed (`cargo run -p xtask -- pins --check | --survey | --fix <case>`,
record: `tests/PIN_HOSTS.toml`); linux sibling re-recording is per-family follow-up work,
gated by `PIN MISSING FOR HOST` findings. Written 2026-11-25 on branch `tier1-pins17`
(leaf for issue #17; main 8ed5f91).

## 0. The problem, precisely

Two different things got conflated during the Nov 2026 CI campaign:

1. **Live differential assertions** (`assert_differential` in
   `crates/ap-cli/tests/stdlib_modules_differential.rs`) run the **host's own
   CPython** beside antipythony and compare the two. These self-adapt: on a
   linux runner the reference *is* linux CPython, so an errno or libm
   divergence between hosts never appears — only a real gap between our impl
   and the local CPython does. The three suites fixed in `b849c9c` were
   exactly this kind (see §3).

2. **Frozen textual pins** — `tests/corpus/**/expected/*.{out,err,rc,stdout,
   stderr,exit}` triples (recorded by test-track authors against
   `/opt/homebrew/bin/python3.14` 3.14.7 on the darwin-aarch64 dev box, with
   `.meta` provenance from `tests/tools/_gen_meta.py`), `tests/ce-s1/expected/
   *_cpython314_stdout.txt`, `tests/t04_ir_verify/fixtures/expected_t04.txt`.
   These embed whatever the recording host's CPython printed. On the
   x86_64-linux CI runner they are compared against *linux* CPython (or
   against our impl), and any host dialect baked into them becomes a false
   red — or was silently patched out with host-conditional expectations,
   which is the failure mode issue #17 exists to end.

The fix is not "make pins host-blind". Some divergences are **legitimate per-
platform ABI facts** (CPython itself differs per host there); for those we
want *explicit per-host pins* with a mechanical completeness rule. Others are
**Class-X byte-divergences** (AGENTS.md): our implementation's gap, which must
match CPython on EVERY host and must never be "fixed" by a second pin.

## 1. Survey: every darwin-frozen expectation found by grep (2026-11-25)

Scan method: `xtask pins --survey` (mechanical inventory: 884 pin files /
298 cases across the three scanned trees) plus targeted greps for host
dialect markers (`/Users/`, `/opt/homebrew`, `Cellar`, `Darwin`, `Clang`,
errno/surface names) over the pin contents. Classification per AGENTS.md:

* **ABI-LEGIT** — reference CPython itself differs per host (errno numeric
  values, epoll/kqueue attribute surfaces, libm version rounding). A pin
  freeze here is host-dependent BY CONTRACT → needs a `<key>` sibling pin
  per host that runs the suite; `--check` polices that completeness.
* **CLASS-X** — real antipythony implementation gap; the expectation must
  match CPython on every host. No pin can fix it; route to the owning
  impl leaf.
* **HOST-INDEPENDENT** — bytes are identical on both hosts (either genuinely
  host-free, or host-bearing text that every consumer suite masks identically
  on both sides of the diff — `_mask_root` family). One pin serves all hosts;
  declaring it in `tests/PIN_HOSTS.toml` puts its hash under --check on both.

| Pin / area | What diverges | Classification | Notes |
|---|---|---|---|
| `tests/corpus/threading/expected/T109-exception-in-thread.stderr` | Traceback frames reference the interpreter's OWN stdlib install: `/opt/homebrew/Cellar/python@3.14/3.14.7/Frameworks/.../threading.py` (absent on Ubuntu, which uses `/usr/lib/python3.14/threading.py`). Line numbers differ too (Apple patches). `_mask_root` only collapses `tests/corpus/` prefixes → the pin fails verbatim on linux. Consumer: `tests/t10_threading` reference-face diff. | **ABI-LEGIT** (host path/line dialect) | manifest case `T109-exception-in-thread` — needs `linux-x86_64` sibling pin (record on the CI leg with `pins --fix T109-exception-in-thread`). |
| `tests/corpus/stdlib/expected/s006_math_basics.out` | repr tails of `lgamma`/`erf`/`expm1` rows bake the darwin libm rounding dialect (the exact class `b849c9c` had to work around for the live differential: glibc `log@GLIBC_2.29`, fused-mul_add shifting lgamma(5) by 1 ulp). | **ABI-LEGIT** (libm 1-ulp dialect) | manifest case `s006_math_basics`; linux sibling pin pending. Honest caveat: "ABI-LEGIT" here records that *reference-vs-reference* differs; if our impl lands a third value on one host, that IS Class-X — the per-host pins make the two cases distinguishable. |
| `tests/corpus/stdlib/expected/s007_math_edges_errors.out` | Same dialect class (nextafter/ulp rows). | **ABI-LEGIT** | manifest case `s007_math_edges_errors`. |
| `tests/corpus/stdlib/expected/s003_os_lite.out` | `os.name == 'posix'`, sep `'/'` rows — identical on linux. | **HOST-INDEPENDENT** | no manifest case needed; .meta provenance carries the record. |
| `tests/corpus/stdlib/expected/s040_stat_errno.out` | errno *names* (ENOENT/EEXIST/EACCES) only — name→value numbers are host-ABI, values are not printed. | **HOST-INDEPENDENT** | would become ABI-LEGIT if a re-record ever prints numeric values (the `a1_err0_table` shape below). |
| `tests/test_a1_rustpython_backfill.py` inline pins (`EXPECTED["a1_err0_table"]`: `EAGAIN 35`, `EDEADLK 11`, `ENOTSUP 45`/`EOPNOTSUPP 102`, `len 107`, `errorcode` last-writer; `a1_kill0` `bad sig errno: 22`; `a1_pipe1` pipe2-absent self-skip; `a1_stat1` `UF_/SF_` + `UF_SETTABLE` darwin-only constants) | The file's own header says the pins are "pinned by a real /opt/homebrew/bin/python3.14 run on macOS (darwin arm64)". On linux CPython: `EAGAIN/EWOULDBLOCK=11`, `EDEADLK=35`, `EILSEQ=84`, `ENOTSUP=95`, `EQFULL=130`, `len(errorcode)=134`, `os.pipe2` EXISTS, no `UF_/SF_`. | **ABI-LEGIT** as *expectations* — and note `b849c9c` already made the IMPLEMENTATION emit per-host values matching linux CPython, so these inline pins go red on a linux pytest leg. | IN-PYTHON pins (not files) → `xtask pins` cannot police them; they need per-host tables in the test file itself (a `sys.platform`-keyed EXPECTED), which is the sibling-leaf pattern the issue's "linux sibling handling" asks for. Recorded here as survey backlog — this leaf does not edit `crates/ap-cli/tests/*` or sibling-owned suites. |
| `tests/test_a12_mined_corpus.py` (`realpath_edges`: expected `/Users/caimlas/git/antipythony`; `predicates_common`: `isdir('/tmp')==True`, `exists('/etc/hosts')==True`; `kill_semantics` errno 22) | cwd-dependent absolute path + filesystem-presence expectations from the darwin box. `/etc/hosts` also exists on Ubuntu (claim survives); `realpath('')` pins THIS checkout path (worktree-fragile, darwin-or-not). | **HOST-INDEPENDENT** (kill_semantics: EINVAL=22 on both → genuine ABI-free) / **env-fragile, not host-ABI** (realpath, predicates) | xfail-soft or already-green rows; the realpath row is a test-authoring bug class (pin cwd-dependent bytes), flagged for its owner, not pin-tool work. |
| `tests/corpus/stdlib/expected/s016_collections_abc_isinstance.err`, `s023_ast_parse_dump_unparse.err`, `tests/corpus/fstrings/expected/f33_bytes_adjacent_error.err` | Absolute recording-checkout path (`/Users/caimlas/git/antipythony/...`) inside warning/traceback lines. | **HOST-INDEPENDENT (post-mask)** — every consumer (`t11`, `t08`, `fstrings`, `t10` suites) runs BOTH sides through `_mask_root` (`/...tests/corpus/... → <ROOT>/tests/corpus/...`), so the dialect cancels identically on any host. | declared `host_independent = true` in the manifest with hashes → --check polices their bytes on both hosts, no sibling pins. |
| `tests/ce-s1/expected/*_cpython314_stdout.txt` (containers/longgc/methodmem/pydantic_diff) | None found — pure-Python/C-API value tables compared live against the host CPython by `crates/ap-ce/tests/ce_s1_*_units.rs`. | **HOST-INDEPENDENT** | hash-pinned in the manifest anyway (cheap regression tripwire). |
| `tests/corpus/exceptions/e016..e020_*_instrument_lines.py` (grep hit for "Darwin") | The match is source-comment prose about instrument lines, not a pinned expectation; expected triples are path-masked like s016. | **HOST-INDEPENDENT** | no action. |
| `tests/t04_ir_verify/fixtures/expected_t04.txt` | Row data is host-free; the HEADER records the darwin reference identity (`[Clang 21.0.0...] /opt/homebrew/...`), which reads as a provenance claim that's false on linux. | **ABI-LEGIT** (provenance-header only) → re-record via `gen_expectations.py` on linux; tool-internal `# pin-host:` marker then makes it explicit. | manifest case `expected_t04`. |
| `plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt`, `crates/ap-run/tests/re_fuzz.rs` / `crates/ap-obj/tests/repr_fuzz.rs` FUZZ_SEEDS | Seeds are byte-level regex inputs replayed against the LIVE host reference (differential, not frozen expectations). | **HOST-INDEPENDENT** (mechanism already host-adaptive) | no pins; nothing to do. |
| `crates/ap-cli/tests/stdlib_modules_differential.rs` `# PROBED python3.14-darwin` comments (select surface line ~950) | Comment-only provenance of a live differential probe (the impl gate is platform-branched per `b849c9c`). | **HOST-INDEPENDENT** assertions; comment is a provenance note | the standardized marker scheme (§2) replaces ad-hoc `PROBED python3.14-darwin` prose for future files; existing comments left untouched (sibling-owned file). |
| docs gate tables (e.g. `docs/rustpython-leaf-donor-inventory.md` gap tables) | informational prose | **out of scope** for the tool (per task brief) | — |

Survey mix over the 17 host-dialect-bearing items probed above:
**ABI-LEGIT 5** (T109.stderr, s006, s007, a1 inline errno/stat/pipe2 block,
expected_t04 header) · **HOST-INDEPENDENT 9** (s003, s040, s016/s023/f33
masked trio, 4× ce-s1, e016–e020 comments, FUZZ_SEEDS/probe-comments/docs
grouped) · **CLASS-X 0 among frozen pins** — the campaign's Class-X findings
were all *live-assertion* bugs (fixed in `b849c9c`), and every frozen pin
either masks or genuinely matches; that is the important negative result: no
frozen pin was papering over an implementation gap. Two near-misses are
recorded honestly: the a1 inline pins would surface `b849c9c`'s per-host
errno table as red-on-linux in pytest (expectation-side dialect, sibling
backlog), and `realpath_edges` pins a cwd-dependent path (test-authoring
bug, neither ABI-LEGIT nor CLASS-X).

## 2. The scheme

**Pin key** = `<os>-<arch>`, normalized by `xtask pins`: today's relevant
space is `darwin-aarch64` (dev box, Homebrew CPython 3.14.7) and
`linux-x86_64` (CI runner, ubuntu-latest). `host_key()` aliases
`macos→darwin`, `arm64→aarch64`, `amd64→x86_64`.

**Markers, two layers (the tool reads both):**
* `.meta` provenance sidecars (`tests/tools/_gen_meta.py`) — already cover
  879/884 corpus pin files; their `sys_version` toolchain string is
  classified (`Clang|Apple → darwin`, `GCC|GNU → linux`) into a host key.
* For pin files with no `.meta` (ce-s1, t04 fixtures, future generated
  pins): the standardized header line, first three lines of the file —

      # pin-host: darwin-aarch64 cpython 3.14.7

  — which `pin_host_header()` parses. New pin generators emit it; it replaces
  ad-hoc prose like `PROBED python3.14-darwin`.

**`tests/PIN_HOSTS.toml`** is the record. Each `[[case]]`: `name`,
`family`/`expected_dir`, `hosts = ["<key>", or "<key>:<sha256>"]`,
`host_independent` / `host_dependent` (mutually exclusive claims),
`source` (the script `--fix` re-runs), `note`. A case with neither a
manifest entry nor any marker is **unmarked** — `--survey` counts them
(backlog), `--check` keeps them informational so the gate can land green.

**`--check` semantics (current policy, exit codes verbatim in §4):**
1. recorded hash (host key == current host, or `host_independent` primary
   channel) ≠ file bytes → `PIN DRIFT` → exit 1;
2. manifest case resolves to no files → `UNKNOWN CASE` → exit 1;
3. `host_dependent` case whose `hosts` omits the current host →
   `PIN MISSING FOR HOST <key>` → exit 1 — but only for hosts listed in
   `known_hosts` (transitional: right now only `darwin-aarch64`, because
   no linux sibling pins have been recorded yet; `--host linux-x86_64`
   demonstrates the four expected missing-sibling findings);
4. unmarked pins: counted, reported, non-failing.

**`--fix <case>` flow:** discover reference (`AP_CONFORM_REFERENCE_PYTHON`
→ `python3.14` on PATH, `--version` must report 3.14 — the ap-conform
contract) → re-run `[[case]].source` from the workspace root (relative
script path so traceback shapes match the t11 recording convention) →
rewrite the case's triple (`.out/.err/.rc` or `.stdout/.stderr/.exit`),
collapsing absolute `…/tests/corpus/…` paths to `<ROOT>/tests/corpus/…`
exactly like consumer `_mask_root` → print the new primary sha + host key.
It never touches `.meta` files (re-run `_gen_meta.py` for provenance) and
never regenerates more than the one named case (corpus-grows rule; mass
re-pinning stays a per-family, owner-gated campaign). Verified smoke on
2026-11-25: `pins --fix demo` against a temp manifest entry re-ran
CPython and rewrote the three channels byte-correctly.

**CI matrix plan** (for the campaign leaf that wires it; this leaf only
documents, since ci.yml is off-limits here):
* PR gate (ubuntu runner): `cargo run -p xtask -- pins --check` with
  `known_hosts = ["darwin-aarch64", "linux-x86_64"]` once sibling pins
  land; linux legs compare against **linux pins** only.
* macOS main-push leg (the `conformance-pytest (macos-latest)` matrix leg
  ci.yml already defines): keeps **darwin pins**; the darwin-aarch64 host
  key is what `--check` auto-detects there.
* Recording a missing sibling: run `pins --fix <case>` on that host, paste
  the printed `"<key>:<sha>"` into the case's `hosts`, re-run the family
  suite there to prove the live diff is green, commit.
* Class-X guard: a case whose two host pins ever differ on bytes that are
  NOT one of the dialect families in §1 (errno numbers, select surface,
  libm repr tails, interpreter install paths) is the signal to stop
  re-pinning and file the impl bug instead — two pins must never freeze
  a divergence our runtime should not have.

## 3. How b849c9c's three suites relate (and why they need NO pins)

`b849c9c fix(stdlib): linux parity for errno/select/math` changed
`crates/ap-run/src/builtins_modules.rs` (errno platform-gated table,
select EPOLL*/KQ_* surfaces, math cbrt/lgamma glibc symbol routing) so the
**implementation** matches the **live host CPython** in
`stdlib_modules_differential::{errno_full_table, math_basics,
select_unsupported_platform}`. Those assertions run the host's reference
beside us and byte-compare at test time — after the fix they are
*host-true implementations*: the correct linux answer is computed, not
recorded, so there is nothing to pin. (The commit message records the
discipline this doc generalizes: gating the probes to darwin was tried
first and reverted in `c7be262` — wrong, because a live differential that
is red on a host is a Class-X bug, never a stale pin.) The frozen-textual
suites in §1 are the complement: they CAN'T be host-true at assertion
time, so they get explicit per-host pins plus the completeness rule.

## 4. Honest scope — what --check does NOT police yet

* **Inline Python pins** (`test_a1_rustpython_backfill.EXPECTED`,
  `test_a12_mined_corpus` param strings): not files → invisible to the
  scanner. Their ABI-LEGIT rows (errno table, UF_/SF_, pipe2) need
  `sys.platform`-keyed tables in the owning suites.
* **Existing test execution is untouched** — the 879 .meta-bearing corpus
  pins still compare raw bytes under each suite's own masking today;
  `--check` adds a *record* gate (hash drift + host coverage), not a new
  diff path. Suites do not yet read PIN_HOSTS.toml.
* **No pin was regenerated or weakened in this leaf** (task constraint:
  "do not weaken/regenerate"). All `hosts` hashes above are the current
  darwin bytes; `known_hosts` stays darwin-only until linux siblings are
  recorded — meaning on a linux host `--check` today reports the four
  expected `PIN MISSING FOR HOST linux-x86_64` findings and exits 1, which
  is the actionable backlog, not a regression.
* Live re-derivation vs the per-host pin is asserted only for the
  `host_independent` hash set and the declared dialect cases; the remaining
  291 corpus cases rely on their recorded `.meta` + suite masks.
* `--fix` covers corpus triples (`.out/.err/.rc`, `.stdout/.stderr/.exit`)
  only — single-`.txt` pins (ce-s1, t04) get re-recorded by their owning
  generators (`ce_s1` drivers, `gen_expectations.py`), consistent with the
  provenance-lint contract in `tests/corpus_traceability`.
* The pin-completeness rule is keyed to `known_hosts` (transition dial);
  once the campaign flips it to include linux-x86_64 it can be removed for
  an unconditional rule.
