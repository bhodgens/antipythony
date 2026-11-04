"""antipythony deviation shim — cryptography.hazmat.primitives.hashes (v0).

Package (not module) so BOTH forms work:
  from cryptography.hazmat.primitives import hashes   # submodule binding
  from cryptography.hazmat.primitives.hashes import SHA256  # direct import

Classes live in ._mod to avoid the name-shadowing trap (a `hashes` name that
is both a class and a module in the same namespace).
"""

from cryptography.hazmat.primitives.hashes._mod import (
    BLAKE2b,
    BLAKE2s,
    ExtendableOutputFunction,
    Hash,
    HashAlgorithm,
    HashContext,
    MD5,
    SHA1,
    SHA224,
    SHA256,
    SHA384,
    SHA3_224,
    SHA3_256,
    SHA3_384,
    SHA3_512,
    SHA512,
    SHA512_224,
    SHA512_256,
    SHAKE128,
    SHAKE256,
    SM3,
    XOFHash,
)
