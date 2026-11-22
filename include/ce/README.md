# antipythony C-extension headers

`apx_python.h` is the single header a C extension includes to compile
against the antipythony C API (Plan 10). Build with:

    APX_INCLUDE=<this directory> antipythony build-ext hello.c -o libhello.so

The user contract — what the surface provides, the documented S0
divergences, the differential-testing discipline, and the distribution
model — is docs/build-ext.md in the repo checkout (and mirrored below in
source archives). The measured compatibility surface is
docs/ce-compat-survey.md; the S1 scope note is
docs/plan10-1003-s1-feasibility.md.

Binary CPython wheels NEVER load: recompiling against this header is the
only supported path (the locked v2 policy).
