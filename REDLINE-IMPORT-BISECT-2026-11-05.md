REDLINE 2026-11-05 ~20:00 — handed from the meept-cwd session (user directive).

## Redline for the W116/W115 leaf
"import scrapy OOMs at ~53 GB pre-parse." Bisect the IMPORT chain
(w3lib -> codecs/encodings), not the crawl path.

## Evidence (read this before bisecting — it partially contradicts the redline)
1. /tmp/w116_t1cont/run-e6.sh with the freshly rebuilt binary (md5 c3250422,
   19:42, post-bcc0a04/743e277) ran bridge-e6.py: 0 -> 35 GB in ~60s,
   RC=137 PEAK_RSS_KB=55623152 (~53 GB) at 19:57. GC commits did NOT fix it.
2. BUT bridge-e6.py (lines 9-11) does `import scrapy` AND
   `from scrapy.crawler import CrawlerProcess` AND starts a file:// crawl.
   The e6 E-marker stderr trace shows growth AFTER start-yield markers.
   The 53 GB peak is on the combined import+crawl path, so "pre-parse
   import-only" is NOT yet proven by the e6 data.
3. Counter-evidence supporting the bisect anyway: e6c run (19:58:41->19:59:02,
   ~21s) hit 17.2 GB (7840kb -> 17275168kb, ~5 GB per 15s slope) and was
   killed by the NEW FENCE ("FENCE kill at 17275168kb" in e6c.err — the
   kill-on-peak you wanted is already live in run-e6c.sh). If most of that
   17 GB accumulated before E("START-YIELD enter"), it is the import chain.
   Check the timestamp of the START-YIELD marker in e6c.err vs the rss slope.
4. probe-codecs1.py held ~17 MB (healthy) on codecs-lookup-only probes;
   probe-w3lib.py exists for the w3lib.encoding.resolve_encoding step.
   The codecs/encodings shim edit in the WIP diff (vendored_stdlib/
   codecs.py, encodings/__init__.py, encodings/aliases.py) is the prime
   suspect: a registry that re-synthesizes CodecInfo per lookup and pins
   every live object via a growing registry table = leak-immune to GC.

## Suggested bisect ladder (fence every rung, 8 GB kill cap)
R0  print("hi")                          -> expect ~10 MB
R1  import codecs; lookup utf_8 x1000     -> expect flat
R2  import w3lib.encoding; resolve x loops
R3  import encodings, encodings.aliases
R4  import scrapy (no crawl)              -> THE redline rung
R5  import scrapy.crawler (no crawl)
R6  full bridge-e6 crawl                  -> known-bad (53 GB)
First rung that blows = leak owner. Log RSS per rung with ps -o rss=, not
the bare 10s poller (the leak doubles in seconds).

## Rules (unchanged)
- cont-bin at /tmp/w116_t1cont/ is a refuse-guard (exit 126). Real binary
  is cont-bin.QUARANTINED-leaky (f57df72a). Run rebuilt binaries from
  ~/git/antipythony/target/debug/antipythony ONLY under the fenced
  executor (dea6c03 / run-e6c.sh pattern, 8 GB kill-on-peak).
- Never fork scrapy/twisted; fix runtime/shims.
- The 44865 watcher from this dir has exited; no antipythony process is
  running right now (verified 20:00).

## RESULT (bisect executed 20:05-20:15, meept-cwd session, fence 8GB)
Binary used: run116.bin (WIP, md5 e86aa56c) vs head-bin (clean 743e277
worktree, md5 48235da5). Vendored codecs/encodings W113 shims present in
BOTH runs (identical trees; dirty-vendored == worktree == HEAD shim).
- head-bin: probe-import.py -> LookupError cp1252 (clean shim lacks W113
  registry; expected) — NO leak possible on this rung.
- run116.bin + probe-import.py: FENCED at 8.6GB between I6 (w3lib resolve
  ok) and I7 (import parsel).
- run116.bin + 60-alias lookup loop (probe-lookups.py): FENCED at 8.5GB
  after just 56 successful codecs.lookup() calls. ~150MB PER LOOKUP.
LEAK OWNER: the codecs.lookup chain itself — each lookup alias-imports
encodings.<name>, which EXECs a module defining 4-5 Python classes
(IncrementalEncoder/Decoder, StreamWriter/StreamReader) + a CodecInfo
tuple-subclass instance. Class-heavy module exec under the WIP
crates/ap-run/src/ diff (classes.rs container_new_marker, interp.rs
~966-line churn) leaks ~150MB GC-immune per class-set created.
w3lib/parsel/scrapy are downstream AMPLIFIERS (they lookup + exec many
modules), not the origin. R1b (lookup loop) = first blowing rung.
Next fix target: class creation / type object lifecycle in WIP
classes.rs + interp.rs — objects pinned outside the GC heap.
