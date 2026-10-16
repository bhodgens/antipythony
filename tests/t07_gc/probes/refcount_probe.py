# Impl-side probe (NOT a corpus case — lives outside tests/corpus/ so the
# C-19 corpus lint never sees it). Cites: C-19, D3.
# Prints whether the refcount-introspection API exists on this
# implementation. C-19: presence/absence/values are never gated, and no
# conformance case may CALL the API; this probe only checks existence,
# which is the 02.04 design decision ("refcounts are not an observable
# concept" — absence or a stub-safe value are both conformant, a
# reference-identical integer count is NOT possible under a tracing GC).
import sys

print("has_getrefcount:", hasattr(sys, "getrefcount"))
print("has_gettotalrefcount:", hasattr(sys, "gettotalrefcount"))
