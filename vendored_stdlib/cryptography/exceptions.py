"""antipythony deviation shim — cryptography.exceptions (v0).

Surface pinned vs CPython 3.14 (wave-110).
"""

from cryptography import (
    AlreadyFinalized,
    AlreadyUpdated,
    InternalError,
    InvalidKey,
    InvalidSignature,
    InvalidTag,
    NotYetFinalized,
    UnsupportedAlgorithm,
)

rust_exceptions = None  # CPython surface (pyo3 backend module); unused v0
typing = __import__("typing")
