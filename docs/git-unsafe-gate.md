# Git unsafe gate (issue #15 policy)

A lightweight, opt-in git hook pair that blocks **newly added** `unsafe`
usage in Rust sources from being committed without an accountability tag.

## What it does

- `.githooks/pre-commit` runs `git diff --cached -U0 -- '*.rs'`, keeps only
  added (`+`) lines (diff `+++` headers stripped), and counts matches on the
  word `unsafe`. If the count is > 0 it prints the offending
  `file:line: content` (first few per staged .rs file) and **fails the
  commit**.
- Only additions are policed. Deleting or moving-out `unsafe` is always
  allowed (a pure-deletion diff has no `+` content lines). A pure *move* of
  an existing `unsafe` line still trips the gate (it appears as an added
  line) — that is deliberate: use the bypass tag with `reason: move`.
- The hook never runs cargo. It is a grep over the staged diff: < 0.5 s.
- `.githooks/commit-msg` enforces the bypass contract (below). The two hooks
  communicate through a stamp file in the git dir
  (`UNSAFE_GATE_ADDED`), consumed and removed by commit-msg so it can never
  leak into a later commit.

## Opt-in (why it is NOT enabled by default)

This repo is shared with active multi-agent fleet sessions. Enabling repo
hooks silently would gate every sibling's commits mid-campaign, so
enablement is a deliberate, per-repo act by the user:

```sh
scripts/enable-git-hooks    # sets: git config core.hooksPath .githooks
scripts/disable-git-hooks   # reverts (git config --unset core.hooksPath)
```

AGENTS.md records the machine fact behind the mechanics: this host's GLOBAL
`core.hooksPath=~/.githooks` contains the Git LFS hooks only, and a
repo-local `core.hooksPath` silently overrides (disables) hooks resolution
against the global path on this machine.

## LFS interaction on this machine

Because enabling sets a repo-local `core.hooksPath`, git would otherwise
skip the global LFS `pre-commit` for this repo. The gate therefore **chains
LFS itself**: if the global hooksPath exists and its `pre-commit` mentions
`git lfs`, the gate runs it first (falling back to `git lfs pre-commit` when
`.gitattributes` declares `filter=lfs`). `commit`-lifecycle LFS stays intact
while gated; the *other* LFS lifecycle hooks (post-checkout, post-merge,
post-commit, pre-push) do not fire for this repo while enabled —
`scripts/enable-git-hooks` prints this caveat, and
`scripts/disable-git-hooks` restores full global coverage.

## Bypass contract

| Variable / tag | Effect |
| --- | --- |
| `SKIP_UNSAFE_GATE=1 git commit ...` | Exit immediately; gate fully off for this commit (no stamp, no tag needed). For emergencies/measurement runs. |
| `ALLOW_UNSAFE_COMMIT=1 git commit -m "... [unsafe-ok: reason]"` | pre-commit allows the new unsafe, stamps `bypass`; commit-msg then **requires** the message to contain `[unsafe-ok: <non-empty reason>]` and fails otherwise. This is the accountability path. |

Note `git commit --no-verify` bypasses both hooks entirely — it exists, but
it defeats the policy and is not part of the contract; CI review (not hooks)
remains the real gate per AGENTS.md ("Repo gates run in CI, NOT git hooks").

## Relationship to `scripts/unsafe_report.sh --diff`

That ratchet script (sibling work for issue #15 assessment) counts/compares
unsafe usage over time. This hook deliberately does **not** call it: the hook
must stay fast, dependency-free, and self-contained (inline grep), so a
missing or mid-development report script can never break a developer's
commit. The two are complementary: the hook blocks at commit time; the
report script measures the stock over time.

## Testing without enabling

The hooks are plain scripts; exercise them directly against staged changes
without touching `core.hooksPath`:

```sh
git add scratch.rs && bash .githooks/pre-commit; echo $?   # 1 if unsafe added
```
