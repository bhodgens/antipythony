# Import-chain leak recheck — 2026-11-05 (W116 recheck leaf)

Status: **leak NO LONGER REPRODUCES on the current tree** (verified 21:15–21:19).
The REDLINE bisect's owner attribution (WIP `classes.rs`/`interp.rs` class-creation
diff) is **REFUTED**. The real leak source was the uncommitted **W114 vendored
`_ast.py`/`ast.py` shim** (module-import-time AST class cloning), and it was
fixed live by the sibling W114 session while this recheck was running
(`_ast.py`/`ast.py` rewritten ~21:02–21:10; both binaries are now clean).

## 0. Hard-rule compliance
- Every antipythony run went through a fenced executor (`/tmp/w116_t1cont/run-import.sh`
  or its copy `run-fenced.sh`: `ps` RSS poll every 2s, 8GB kill cap, 240s watchdog).
- No product/stdlib file was modified, reverted, stashed, or committed by this leaf.
  (`ast.py`/`_ast.py` edits at 21:0x were the sibling W114 session's own work.)
- Site-packages confirmed present: `w3lib 2.5.0`, `parsel 1.12.1`, `scrapy 2.19.0`,
  `twisted 26.4.0` under `/Users/caimlas/.cache/antipythany/site-packages/`.

## 1. Rung 1 — rebuild, md5s
`cargo build -p ap-cli --bin antipythony` → all crates **Fresh**, binary
byte-identical to `run116.bin`:

| artifact | md5 |
|---|---|
| fresh `target/debug/antipythony` (recheck build) | `c3250422bd3de747b891638c319a53e4` |
| `/tmp/w116_t1cont/run116.bin` (19:42 bisect build) | `c3250422bd3de747b891638c319a53e4` |
| `/tmp/w116_t1cont/head-bin` = `wt-head/target/debug/antipythony` (clean 743e277) | `48235da534eb990371c96b676522f6c0` |

**So "fresh" and run116 are THE SAME BINARY — there was no rebuild delta.**
Note: the REDLINE/HANDOFF text cites run116 as `e86aa56c…`; the actual file hashes
`c3250422…` (matching REDLINE §Evidence ¶1 "md5 c3250422, 19:42"). The `e86aa56c`
label in the RESULT section is a doc inconsistency, not a different artifact I
could find. Source mtimes unchanged since the bisect for the Rust side
(classes/interp/importer.rs 16:53; codecs.py 19:44) — except the W114 pair
`ast.py`/`_ast.py`, which changed under us at 21:05–21:10 (see §4 timeline).

## 2. Rungs 2–4 re-run at 20:39–21:0x (pre-fix stdlib window): leak DID reproduce

With `import ast` still pulling in the old W114 `_ast.py` shim, every chain rung
blew — **against the live main-tree stdlib, with both binaries**:

```
== rc1-fresh-lookups-cur      (run116.bin + probe-lookups.py)
FENCE kill at 10320848kb
RC=137 PEAK_RSS_KB=10320848
A0 ready
A1 lookups done 56            <-- 56 lookups themselves did NOT leak; blow was in the
                                 A1->A2 segment (import parsel)
== rc3-fresh-parsel-only      (run116.bin + probe: `import parsel` alone)
FENCE kill at 8722848kb
RC=137 PEAK_RSS_KB=8722848
Q0 start                      <-- never reached Q1

== rc4-fresh-import-chain     (run116.bin + probe-import.py)
FENCE kill at 8719696kb
RC=137 PEAK_RSS_KB=8719696
I1 codecs / I2 encodings / I3 lookup cp1252 ok / I5 w3lib / I6 resolve cp1252
                              <-- blows between I6 and I7 (import parsel), as in the redline

== rc5-headbin-parsel-curstdlib  (CLEAN head-bin + same stdlib)
FENCE kill at 8786416kb
RC=137 PEAK_RSS_KB=8786416
Q0 start                      <-- *** HEAD BINARY LEAKS TOO *** refutes "binary is the variable"

== rc27-stdlibs (run116.bin + import dataclasses; unicodedata; string)
FENCE kill at 10464416kb — blew at the FIRST marker (dataclasses)

== final4-ast-only-OLDstdlib  (run116.bin + one line: `import ast`; stdlib snapshot
                                taken 20:44, md5 _ast.py=bade0f45ce1f)
FENCE kill at 9050736kb
RC=137 PEAK_RSS_KB=9050736
== final5-ast-only-OLDstdlib-headbin  (CLEAN head-bin, same snapshot)
FENCE kill at 9175552kb
RC=137 PEAK_RSS_KB=9175552
== final6-ast-only-LIVEstdlib (run116.bin, post-fix stdlib)
RC=0 PEAK_RSS_KB=8000
OK1 ast
```

**First blowing rung (minimal): `import ast` itself.** w3lib/parsel/scrapy are
downstream because they transitively import `ast` (via `dataclasses` — w3lib._url
imports dataclasses at module level; that is why `import w3lib.url`/`w3lib.html`/
`parsel.utils` each blew at ~8.8–10.4 GB while `import w3lib`, `cssselect`, `lxml`,
`jmespath`, `urllib.parse`, `ipaddress`, `pathlib`, `base64` were all flat:

```
== rc22-w3lib.html    FENCE kill at 8957744kb RC=137
== rc26-probe-r-G5-w3liburl-int  FENCE kill at 10421024kb RC=137   (w3lib._url: imports dataclasses)
== rc12-w3lib         RC=0 PEAK_RSS_KB=8832   OK w3lib             (package init: no ast)
== rc19-lxml          RC=0 PEAK_RSS_KB=8832   OK lxml
```

### Mechanism (pinpointed, read-only)
The pre-fix W114 `vendored_stdlib/_ast.py` (757-line snapshot) contained a
~110-name attribute-binding loop working around a real runtime bug:

> `# W114: from _ast import * clones class objects in this runtime (identity
> breaks isinstance across modules). Bind every _ast name by attribute access
> instead` … `globals()[_n] = getattr(_ast_mod, _n)` for 110 names.

`_ast_mod` is the **native** `_ast` bridge module (Rust, `ast_bridge.rs`). Every
`getattr` on it **materializes a fresh clone of the AST class set outside the
GC-managed heap** — ~110 class clones × ~150MB/clone ≈ the 9–10GB fences. This is
consistent with the redline's "~150MB per class-set, GC-immune" measurement but
the class-sets came from the AST bridge, **not** from `encodings.<name>` module
exec. The clone behavior itself is in the committed core (head-bin leaks on the
old stdlib too) — a HEAD-level `ap-run`/`ap-obj` type-identity bug that the W114
shim's `import ast` path triggers; the shim was the accelerant.

## 3. Rungs 2–3 re-run at 21:15–21:19 on the CURRENT tree: CLEAN

Stdlib captured immediately before/after (unchanged across all three rungs):
`ast.py=05cdccea1ef6…`, `_ast.py=e111ec7036b6…` (the sibling's rewritten
"`_ast`: real AST node classes over the native generic tree", 890 lines).

```
== final1-lookups (run116.bin + probe-lookups.py)
RC=0 PEAK_RSS_KB=512
A0 ready / A1 lookups done 56 / A2 parsel ok / ALLDONE
== vfinal1-lookups  RC=0 PEAK_RSS_KB=8608  (same, repeat)  ALLDONE
== vfinal3-parsel   RC=0 PEAK_RSS_KB=9536   import parsel alone  QDONE
== final2-full-import-chain (run116.bin + probe-import.py I0–I8)
RC=0 PEAK_RSS_KB=843504
I1 codecs / I2 encodings / I3 lookup cp1252 ok / I5 w3lib / I6 resolve cp1252 /
I7 parsel / I8 scrapy / ALLDONE              <-- FULL SCRAPY CHAIN COMPLETES
== vfinal2-fullchain RC=0 PEAK_RSS_KB=879344  ALLDONE (repeat)
```

Verdict per the gate definition ("rung passed = ALLDONE + peak < 200MB"):
- 60-alias lookup loop: **PASS** (0.5–8.6MB, ALLDONE, 56/56 succeed).
- `import parsel` alone: **PASS** (~9MB).
- Full scrapy chain I0–I8: **ALLDONE at 843–879MB** — under the HANDOFF's
  1.5GB scrapy-chain target, above the 200MB micro-rung bar; this is the
  known "single scrapy import ≈ 0.8GB in debug builds" footprint
  (~270 modules re-parsed per process, see NEXT-STEPS-W112 note), not a leak —
  it is flat across repeats and the clean e6h crawl peaked 1.36GB.

## 4. Timeline (why the two sessions disagreed)
| time | event | evidence |
|---|---|---|
| 19:42 | run116.bin built (W114 `_ast.py` shim in vendored tree) | md5 c3250422 |
| 20:05–20:15 | redline bisect: lookups+parsel & I6→I7 fence-kills; blames WIP classes.rs/interp.rs | imp-wip.log, imp-lookups.log |
| ~20:25 | sibling "clean 17MB" lookups-loop observation | lookups WITHOUT the trailing `import parsel` segment don't touch `ast` → flat; this also explains my rc2 (lookups-only, 8.6MB flat, PRE-fix). "Rung ordering" resolved: ordering only mattered insofar as a rung imported `ast`. |
| 20:39–21:01 | this recheck, pre-fix window: all ast-reaching rungs blow on BOTH binaries | rc1–rc29, final4/final5 |
| 21:02–21:10 | sibling W114 session rewrites `vendored_stdlib/_ast.py` + `ast.py` (mtimes 21:05/21:06/21:10) | stat + md5 drift snapshots (`bade0f45→6208f008→05cdccea`, `_ast bade0f45→e111ec70`) |
| 21:11–21:19 | every rung ALLDONE, full scrapy chain 843–879MB, `import ast` = 8MB on both binaries | now1/now2/final1/final2/vfinal1–3/final6 |

So the 20:25 discrepancy was neither tree-staleness nor rung-order alone: the
20:10 rung and the 20:25 probe were different probes (probe-lookups.py itself was
edited at 20:28:51), and the leak moved in lockstep with the **stdlib file
contents**, which were being rewritten underneath both sessions.

## 5. cp932 LookupError — NOT a WIP regression; native multibyte codec gap
- CPython reference: `codecs.lookup('cp932')` → `cp932` (verified: python3 3.15 stdlib;
  CPython 3.14 per project gate does the same).
- Pinpoint probe (run116.bin, live stdlib):
  ```
  C0 codecs imported
  C1 _codecs_jp import: ModuleNotFoundError No module named '_codecs_jp'
  C2 lookup cp932: LookupError unknown encoding: cp932
  C3 import encodings.cp932: ModuleNotFoundError No module named '_codecs_jp'
  CDONE
  ```
- Failure chain: `encodings.aliases` maps `'ms932'/'mskanji'/'932'/'windows_31j' → 'cp932'`
  → `vendored_stdlib/encodings/cp932.py` line 7: `import _codecs_jp` +
  `import _multibytecodec as mbc` → **neither module exists natively** (grep
  `_multibytecodec|_codecs_jp` over `crates/**` = zero hits; no vendored pure-Python
  fallback either) → `codecs._alias_import`'s documented `except Exception: return
  None` (APX-W113 comment: "multibyte codecs … import the native _multibytecodec
  module, which antipythony does not provide … Surface those as LookupError") →
  `LookupError: unknown encoding: cp932`.
- Scope: the whole multibyte family — cp932/cp949/cp950/big5/big5hkscs/euc_jp/
  euc_kr/euc_jisx0213/gb18030/gb2312/hz — plus the 4 probe-visible misses
  (`big5_tw, csbig5, big5_hkscs, hkscs` failed even in the clean rc2/final1 runs).
  This is pre-existing W113-documented missing native module surface, NOT a
  Class-S regression introduced by the WIP diff. It can bite scrapy's
  `response.text` for CJK pages via w3lib `resolve_encoding` (which catches
  LookupError) — real gap, separate ticket, different owner (`_multibytecodec`
  + `_codecs_jp/kr/tw/cn` native modules, i.e. Plan-02.08 module surface).

## 6. Probable owner (verdict)
1. **Leak owner (closed by sibling during this recheck):** the W114 uncommitted
   `vendored_stdlib/_ast.py`/`ast.py` shim + the underlying HEAD-level
   native-`_ast` `getattr` class-clone behavior in the Rust AST bridge
   (`crates/ap-run/src/ast_bridge.rs` / type-identity core). Fix = sibling's
   `_ast.py` rewrite (real AST node classes over the generic tree, no per-name
   clone loop). NOT `classes.rs container_new_marker` / `interp.rs` churn —
   the redline's conclusion was a misattribution caused by (a) only ever varying
   the binary while the stdlib was the actual variable, and (b) reading the
   fence in the A1→A2 (parsel) segment as "per-lookup leak".
2. **Residual follow-up:** the native `getattr`-clone behavior itself should be
   fixed in the core (the old shim merely stopped *triggering* it); if any other
   stdlib/site module star-imports or bulk-getattrs native type modules, the
   same GB-scale clone leak can return. Recommend a regression probe keeping
   `import ast; import dataclasses; import w3lib._url` RSS-gated in CI.
3. cp932 family: owner = missing native `_multibytecodec`/`_codecs_*` modules
   (2026-11-05 recheck confirms it predates and is independent of the WIP diff).

## 7. Artifacts
- Fenced logs: `/tmp/w116_t1cont/fence-rc*.log`, `fence-final*.log`, `fence-vfinal*.log`,
  `fence-now*.log` (all RC/PEAK_RSS lines quoted above are verbatim).
- Probes: `probe-lookups.py` (20:28 version, 56-alias loop + parsel),
  `probe-import.py` (I0–I8), `probe-recheck-parsel-only.py`, `probe-r-J-ast.py`,
  `probe-r-M/L-*`, `probe-cp932-pinpoint.py`.
- Stdlib snapshots: `current-vendored/` (20:44, leaky `_ast`=bade0f45),
  `snap2110/` (21:09, transitional), `wt-head/vendored_stdlib/` (HEAD stub shim).
- No repo files modified by this leaf except this doc.
