# Branch Protection - Recommended GitHub Settings (leaf 00.04)

Human-facing doc: enabling branch protection is a GitHub UI/API action, not a
code change. Apply these settings for `main` on the antipythany repo after
`.github/workflows/ci.yml` runs green at least once remotely.

## Required status checks

Every job in ci.yml must be green before a merge to `main`:

- `fast lints (traceability + notices)` - corpus-citation lint, golden-
  provenance lint, THIRD_PARTY_NOTICES drift, vendored-stdlib drift.
- `rust 1.94.0 (fmt / clippy-info / build / test)` - fmt blocking, clippy
  informational, workspace build+test, cargo-doc gate, cli-docs drift.
- `cargo deny (master.md:45; finding #2)` - license allow-list gate.
- `conformance os-axis` + `conformance (ubuntu-latest)` and
  `conformance (macos-14)` on main pushes - the byte-exact differential suite
  plus the XPASS ceiling gate.

Do NOT allow bypass. Do not add `nightly-fuzz` here - it is a scheduled
workflow, not a merge gate.

## Rules to enable

1. Constraint: "Require a pull request before merging" (min 1 approval for
   humans; agent-wave commits on a campaign branch may merge via orchestrator
   review per AGENTS.md convention - if that conflicts, use "Require
   review from code owners" off and keep 1 approval).
2. "Require status checks to pass" -> select the checks above by name.
3. "Require branches to be up to date before merging" -> on.
4. "Do not allow bypassing the above settings" -> on (admins included).
5. "Allow auto-merge" -> on (rebase merge) if you want queued leaves to land
   serially; rebase keeps the linear history the plan tree assumes.
6. Block force pushes and branch deletion on `main`.

## Apply via gh CLI

Replace OWNER/REPO with the actual remote path (`git remote get-url origin`).

```
gh api -X PUT repos/OWNER/REPO/branches/main/protection \
  -H "Accept: application/vnd.github+json" \
  -F "required_status_checks.strict=true" \
  -F "required_status_checks.contexts[]=fast lints (traceability + notices)" \
  -F "required_status_checks.contexts[]=rust 1.94.0 (fmt / clippy-info / build / test)" \
  -F "required_status_checks.contexts[]=cargo deny (master.md:45; finding #2)" \
  -F 'required_pull_request_reviews.required_approving_review_count=1' \
  -F 'enforce_admins=true' \
  -F 'allow_force_pushes=false' \
  -F 'allow_deletions=false'
```

Note: `conformance (macos-14)` only exists on main pushes (OS-axis job), so
it cannot be a PR required check; ubuntu conformance covers PRs and macOS
proves the dev-platform parity on main. If the runner tier lacks
macos-14 minutes, delete the macos leg from ci.yml's axis script instead of
disabling the check.

## Verification after enabling

1. Push a branch, open a PR, confirm the checks list shows all four and the
   merge button is blocked while red.
2. Break one gate deliberately on a scratch PR (e.g. a stale golden citation)
   and confirm red blocks merge.
3. With ci.yml green remotely: retire `.github/workflows/attribution.yml`
   (its notices gate is duplicated in fast-lints; robustness-ci leaf 00.01
   handoff note).
