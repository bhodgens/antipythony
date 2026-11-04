"""antipythony deviation shim — cryptography.hazmat.decrepit.ciphers.algorithms (v0).

Twisted common.py prefers the decrepit TripleDES when present.
"""

from cryptography.hazmat.primitives.ciphers.algorithms import (
    ARC4,
    BlockCipherAlgorithm,
    Blowfish,
    CAST5,
    CipherAlgorithm,
    IDEA,
    SEED,
    TripleDES,
)


class RC2(BlockCipherAlgorithm):
    block_size = 64
