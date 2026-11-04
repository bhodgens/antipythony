"""antipythony deviation shim — cipher algorithms (single source)."""


class CipherAlgorithm:
    pass


class BlockCipherAlgorithm(CipherAlgorithm):
    pass


class AES:
    block_size = 128

    def __init__(self, key=None):
        self.key = key


AES128 = AES
AES256 = AES


class TripleDES(BlockCipherAlgorithm):
    block_size = 64


class Blowfish(BlockCipherAlgorithm):
    block_size = 64


class CAST5(BlockCipherAlgorithm):
    block_size = 64


class Camellia(BlockCipherAlgorithm):
    block_size = 128


class ChaCha20:
    block_size = 64


class IDEA(BlockCipherAlgorithm):
    block_size = 64


class SEED(BlockCipherAlgorithm):
    block_size = 128


class SM4(BlockCipherAlgorithm):
    block_size = 128


class ARC4:
    block_size = 8
