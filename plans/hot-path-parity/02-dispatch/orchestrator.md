# HOT-PATH-PARITY — orchestrator (02-dispatch)

Inline caches first (after nursery merges); typed dict regions gated
on a post-nursery + post-for-loop re-measure.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-inline-caches.md | per-site resolution cache + version-counter invalidation | 01-nursery merged |
| 02-typed-dict-regions.md | typed dict getitem/setitem codegen in tier-1 | 01-nursery + 03-jit-for-loops merged, re-measure shows dict rows still < 1.0x |

## Dispatch protocol

1. inline-caches after 01-nursery merges.
2. typed-dict-regions: re-measure first; dispatch only if dict rows
   remain below 1.0x (the nursery + for-loop wins may shift the
   bottleneck).

## Test Gate

Inline-cache leaf proves invalidation (mutate-after-warm unit test)
+ no-GIL thread probe. Typed-dict leaf proves deopt paths.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-inline-caches.md | PENDING |
| 02-typed-dict-regions.md | PENDING (gated) |
