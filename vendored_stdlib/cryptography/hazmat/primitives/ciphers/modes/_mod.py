"""antipythony deviation shim — cipher modes (single source)."""


class Mode:
    pass


class ModeWithAuthenticationTag(Mode):
    pass


class ModeWithInitializationVector(Mode):
    pass


class ModeWithNonce(Mode):
    pass


class ModeWithTweak(Mode):
    pass


class CBC(ModeWithInitializationVector):
    def __init__(self, iv=None):
        self.initialization_vector = iv


class CFB(ModeWithInitializationVector):
    def __init__(self, iv=None):
        self.initialization_vector = iv


class CFB8(ModeWithInitializationVector):
    def __init__(self, iv=None):
        self.initialization_vector = iv


class CTR(ModeWithNonce):
    def __init__(self, nonce=None):
        self.nonce = nonce


class OFB(ModeWithInitializationVector):
    def __init__(self, iv=None):
        self.initialization_vector = iv


class GCM(ModeWithAuthenticationTag):
    def __init__(self, iv=None, tag=None, min_tag_length=16):
        self.initialization_vector = iv
        self.tag = tag
        self.min_tag_length = min_tag_length


class XTS(ModeWithTweak):
    def __init__(self, tweak=None):
        self.tweak = tweak


class ECB(Mode):
    pass


class BlockCipherAlgorithm(Mode):
    pass


class CipherAlgorithm(Mode):
    pass
