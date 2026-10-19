# Vendored CPython stdlib files - license notice

Files under vendored_stdlib/ are copied from CPython 3.14 (python/cpython,
PSF License Version 2 - see LICENSES/PSF-LICENSE-2.0.txt). PSF license
permits redistribution with the license text preserved and attribution.
Each vendored file keeps its original module docstring; this directory
README plus the preserved LICENSES file satisfy the terms for internal and
external distribution. THIRD_PARTY_NOTICES carries a PSF entry via the
xtask static-notices mechanism (files are not cargo crates, so cargo
metadata does not see them; `cargo run -p xtask -- notices` appends the
entry when vendored_stdlib/ exists).

## Deviations from pristine CPython 3.14.7 (audited 2026-10-18)

Most files are byte-identical to the installed CPython 3.14.7 stdlib.
The exceptions, all runtime-binding adaptations:

- `enum.py`: 1 line — `classdict._member_names` read via `getattr(..., [])`
  fallback (our class-dict instance may lack the attr during partial
  metaclass construction).
- `_collections_abc.py`: ~80 lines — binding shims for registry/ABC
  machinery our runtime exposes differently.
- `_py_warnings.py`: 3 lines — warn-path binding to the native warnings
  surface.
- `inspect.py`: ~16 lines — annotation/FrameInfo shims.
- `annotationlib.py`: REPLACED by a self-declared minimal shim (docstring
  says so): CPython's uses t-strings (PEP 750), outside the v1 grammar.
- `_contextvars.py`, `collections.py`: runtime-binding shims with no
  single CPython counterpart file.
