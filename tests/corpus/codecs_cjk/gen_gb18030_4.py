#!/usr/bin/env python3
"""

REGENERATION PIPELINE NOTE: this script emits single-line Gb4Run literals;
`cargo fmt` reformats them multi-line (that multi-line form is what is
committed). After regenerating, run `cargo fmt -p ap-run` before
committing, or the diff will show 1000+ format-only lines.