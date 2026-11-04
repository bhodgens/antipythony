"""antipythony deviation shim — ciphers.algorithms (v0).

Package so BOTH forms work:
  from cryptography.hazmat.primitives.ciphers import algorithms
  from cryptography.hazmat.primitives.ciphers.algorithms import AES, TripleDES
"""

from cryptography.hazmat.primitives.ciphers.algorithms._mod import (
    AES,
    AES128,
    AES256,
    ARC4,
    BlockCipherAlgorithm,
    Blowfish,
    CAST5,
    Camellia,
    ChaCha20,
    CipherAlgorithm,
    IDEA,
    SEED,
    SM4,
    TripleDES,
)
