# Vendored CPython stdlib files - license notice

Files under vendored_stdlib/ are copied from CPython 3.14 (python/cpython,
PSF License Version 2 - see LICENSES/PSF-LICENSE-2.0.txt). PSF license
permits redistribution with the license text preserved and attribution.
Each vendored file keeps its original module docstring; this directory
README plus the preserved LICENSES file satisfy the terms for internal and
external distribution. THIRD_PARTY_NOTICES gains a PSF entry via the xtask
license_fallbacks mechanism (files are not cargo crates, so cargo metadata
does not see them - the xtask generator must be extended with a
static-notices list. See docs/attribution.md).
