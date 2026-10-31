"""antipythany deviation shim — hashes classes (single source of truth)."""


class HashAlgorithm:
    digest_size = 0


class HashContext:
    def __init__(self, *a, **k):
        raise NotImplementedError("antipythany cryptography shim: hashing")


class Hash(HashContext):
    def __init__(self, algorithm=None, backend=None, *a, **k):
        raise NotImplementedError("antipythany cryptography shim: hashing")


class SHA1(HashAlgorithm):
    digest_size = 20


class SHA224(HashAlgorithm):
    digest_size = 28


class SHA256(HashAlgorithm):
    digest_size = 32


class SHA384(HashAlgorithm):
    digest_size = 48


class SHA512(HashAlgorithm):
    digest_size = 64


class MD5(HashAlgorithm):
    digest_size = 16


class SHA3_224(HashAlgorithm):
    digest_size = 28


class SHA3_256(HashAlgorithm):
    digest_size = 32


class SHA3_384(HashAlgorithm):
    digest_size = 48


class SHA3_512(HashAlgorithm):
    digest_size = 64


SHA512_224 = SHA512
SHA512_256 = SHA512
BLAKE2b = SHA512
BLAKE2s = SHA256
SM3 = SHA256
SHAKE128 = SHA256
SHAKE256 = SHA256
XOFHash = Hash
ExtendableOutputFunction = HashAlgorithm
