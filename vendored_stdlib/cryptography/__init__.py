"""antipythony deviation shim — cryptography (v0: import-success surface only).

WHY: twisted.conch.ssh (keys/common/transport/_kex) imports
cryptography.hazmat.* broadly at module level, and scrapy's ExtensionManager
import chain eagerly pulls twisted.conch (telnet+insults) even with
TELNETCONSOLE_ENABLED=False. The file:// crawl gate performs no SSH/crypto
work, so this shim satisfies every module-level `from cryptography... import`
in the staged tree; operations raise NotImplementedError lazily (same pattern
as OpenSSL/SSL.py and service_identity/exceptions.py).

Surface pinned against live CPython 3.14 (wave-110 probe; never guess surface
lists).
"""

import sys as _sys
import types as _types


# ------------------------------------------------------------------ helpers


def _nope(msg):
    def f(*a, **k):
        raise NotImplementedError("antipythony cryptography shim: %s" % msg)

    return f


class _CryptoOp:
    """Base for op classes: any call raises NotImplementedError."""

    def __call__(self, *a, **k):
        raise NotImplementedError("antipythony cryptography shim: op unsupported")


# ------------------------------------------------------- cryptography (root)


class CryptographyDeprecationWarning(UserWarning):
    pass


class DeprecatedIn36(UserWarning):
    pass


class DeprecatedIn40(DeprecatedIn36):
    pass


class DeprecatedIn41(DeprecatedIn40):
    pass


class DeprecatedIn42(DeprecatedIn41):
    pass


class DeprecatedIn43(DeprecatedIn42):
    pass


class InterfaceNotImplemented(Exception):
    pass


def deprecated(*a, **k):
    def deco(f):
        return f

    return deco


class cached_property:
    def __init__(self, f):
        self.f = f

    def __get__(self, obj, cls):
        if obj is None:
            return self
        v = self.f(obj)
        obj.__dict__[self.f.__name__] = v
        return v


def int_to_bytes(value, length=None):
    """The ONLY cryptography function twisted.conch.ssh.common calls at
    runtime — pure arithmetic (SSH MPInt framing), implemented for real."""
    if isinstance(value, str):
        raise TypeError("value should be an integer")
    if value < 0:
        raise ValueError("value must be non-negative")
    if length is None and value == 0:
        return b"\x00"
    if length is None:
        n = 0
        v = value
        while v:
            n += 1
            v >>= 8
        length = n
    return value.to_bytes(length, "big")


Buffer = object  # typing-only aliases; runtime surface only
Callable = object
Sequence = object


# ------------------------------------------------------------------ exceptions


class _CryptoException(Exception):
    pass


class AlreadyFinalized(_CryptoException):
    pass


class AlreadyUpdated(_CryptoException):
    pass


class InternalError(_CryptoException):
    pass


class InvalidKey(_CryptoException):
    pass


class InvalidSignature(_CryptoException):
    pass


class InvalidTag(_CryptoException):
    pass


class NotYetFinalized(_CryptoException):
    pass


class UnsupportedAlgorithm(_CryptoException):
    pass


# --------------------------------------------------------------------- hashes


class HashAlgorithm:
    digest_size = 0


class Hash:
    def __init__(self, algorithm=None, backend=None, *a, **k):
        raise NotImplementedError("antipythony cryptography shim: hashing")


HashContext = _CryptoOp
ExtendableOutputFunction = HashAlgorithm
XOFHash = Hash


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


# -------------------------------------------------------------- serialization


class Encoding:
    PEM = "PEM"
    DER = "DER"
    OpenSSH = "OpenSSH"
    Raw = "Raw"
    X962 = "X962"
    SMIME = "SMIME"


class PrivateFormat:
    PEM = "PEM"
    DER = "DER"
    OpenSSH = "OpenSSH"
    PKCS8 = "PKCS8"
    TraditionalOpenSSL = "TraditionalOpenSSL"
    Raw = "Raw"


class PublicFormat:
    PEM = "PEM"
    DER = "DER"
    OpenSSH = "OpenSSH"
    Raw = "Raw"
    X962 = "X962"
    UncompressedPoint = "UncompressedPoint"
    CompressedPoint = "CompressedPoint"
    SubjectPublicKeyInfo = "SubjectPublicKeyInfo"


class ParameterFormat:
    PEM = "PEM"
    DER = "DER"


class KeySerializationEncryption:
    pass


class NoEncryption(KeySerializationEncryption):
    pass


class BestAvailableEncryption(KeySerializationEncryption):
    def __init__(self, password=None):
        pass


class SSHCertificate:
    pass


class SSHCertificateBuilder:
    pass


class _Marker:
    pass


SSHCertPrivateKeyTypes = _Marker
SSHCertPublicKeyTypes = _Marker
SSHPrivateKeyTypes = _Marker
SSHPublicKeyTypes = _Marker
SSHCertificateType = _Marker

load_pem_private_key = _nope("key loading")
load_pem_public_key = _nope("key loading")
load_der_private_key = _nope("key loading")
load_der_public_key = _nope("key loading")
load_pem_parameters = _nope("key loading")
load_der_parameters = _nope("key loading")
load_ssh_private_key = _nope("key loading")
load_ssh_public_key = _nope("key loading")
load_ssh_public_identity = _nope("key loading")
ssh_key_fingerprint = _nope("fingerprints")


# ------------------------------------------------------ asymmetric submodules


class Prehashed:
    def __init__(self, algorithm=None):
        pass


decode_dss_signature = _nope("DSS signatures")
encode_dss_signature = _nope("DSS signatures")
decode_rfc6979_signature = _nope("DSS signatures")
encode_rfc6979_signature = _nope("DSS signatures")


def default_backend():
    """cryptography.hazmat.backends.default_backend — inert marker."""
    return _CryptoOp()


class dsa:
    class DSAParameterNumbers:
        pass

    class DSAParameters:
        pass

    DSAParametersWithNumbers = DSAParameters

    class DSAPrivateKey:
        pass

    DSAPrivateKeyWithSerialization = DSAPrivateKey

    class DSAPrivateNumbers:
        pass

    class DSAPublicKey:
        pass

    DSAPublicKeyWithSerialization = DSAPublicKey

    class DSAPublicNumbers:
        pass

    generate_parameters = _nope("DSA generate_parameters")
    generate_private_key = _nope("DSA generate_private_key")


class ObjectIdentifier(str):
    pass


class _Curve:
    """Curve *instance* singleton: twisted keys.py does `ec.SECP256R1()` at
    module level, so each attribute must be a callable producing the
    instance (CPython: class whose ctor takes no args)."""

    def __init__(self, name, key_size):
        self.name = name
        self.key_size = key_size

    def __call__(self):
        return self


class ec:
    ObjectIdentifier = ObjectIdentifier  # hoisted; defined above _Curve
    class EllipticCurve:
        name = ""
        key_size = 0

    class EllipticCurveOID:
        SECP256R1 = "1.2.840.10045.3.1.7"
        SECP384R1 = "1.3.132.0.34"
        SECP521R1 = "1.3.132.0.35"

    class EllipticCurvePrivateKey(EllipticCurve):
        pass

    EllipticCurvePrivateKeyWithSerialization = EllipticCurvePrivateKey

    class EllipticCurvePrivateNumbers:
        pass

    class EllipticCurvePublicKey(EllipticCurve):
        pass

    EllipticCurvePublicKeyWithSerialization = EllipticCurvePublicKey

    class EllipticCurvePublicNumbers:
        pass

    class EllipticCurveSignatureAlgorithm:
        pass

    SECP192R1 = _Curve("secp192r1", 192)
    SECP224R1 = _Curve("secp224r1", 224)
    SECP256R1 = _Curve("secp256r1", 256)
    SECP384R1 = _Curve("secp384r1", 384)
    SECP521R1 = _Curve("secp521r1", 521)
    SECP256K1 = _Curve("secp256k1", 256)
    SECT163K1 = _Curve("sect163k1", 163)
    SECT163R2 = _Curve("sect163r2", 163)
    SECT233K1 = _Curve("sect233k1", 233)
    SECT233R1 = _Curve("sect233r1", 233)
    SECT283K1 = _Curve("sect283k1", 283)
    SECT283R1 = _Curve("sect283r1", 283)
    SECT409K1 = _Curve("sect409k1", 409)
    SECT409R1 = _Curve("sect409r1", 409)
    SECT571K1 = _Curve("sect571k1", 571)
    SECT571R1 = _Curve("sect571r1", 571)
    BrainpoolP256R1 = _Curve("brainpoolP256r1", 256)
    BrainpoolP384R1 = _Curve("brainpoolP384r1", 384)
    BrainpoolP512R1 = _Curve("brainpoolP512r1", 512)

    ECDSA = _nope("ECDSA signing")
    ECDH = _nope("ECDH key exchange")
    derive_private_key = _nope("ec derive_private_key")
    generate_private_key = _nope("ec generate_private_key")
    get_curve_for_oid = _nope("ec get_curve_for_oid")


class ed25519:
    class Ed25519PrivateKey:
        pass

    class Ed25519PublicKey:
        pass


class x25519:
    class X25519PrivateKey:
        pass

    class X25519PublicKey:
        pass


class rsa:
    class AsymmetricPadding:
        pass

    class RSAPrivateKey:
        pass

    RSAPrivateKeyWithSerialization = RSAPrivateKey

    class RSAPrivateNumbers:
        pass

    class RSAPublicKey:
        pass

    RSAPublicKeyWithSerialization = RSAPublicKey

    class RSAPublicNumbers:
        pass

    generate_private_key = _nope("RSA generate_private_key")
    rsa_crt_dmp1 = _nope("RSA CRT")
    rsa_crt_dmq1 = _nope("RSA CRT")
    rsa_crt_iqmp = _nope("RSA CRT")
    rsa_recover_prime_factors = _nope("RSA recover")
    rsa_recover_private_exponent = _nope("RSA recover")


class padding:
    class AsymmetricPadding:
        pass

    class PKCS1v15(AsymmetricPadding):
        pass

    class PSS(AsymmetricPadding):
        max_length = classmethod(_nope("PSS max_length"))

    class OAEP(AsymmetricPadding):
        def __init__(self, *a, **k):
            pass

    class MGF:
        def __init__(self, *a, **k):
            pass

    MGF1 = MGF

    calculate_max_pss_salt_length = _nope("PSS salt length")


class dh:
    class DHParameterNumbers:
        pass

    class DHParameters:
        pass

    DHParametersWithSerialization = DHParameters

    class DHPrivateKey:
        pass

    DHPrivateKeyWithSerialization = DHPrivateKey

    class DHPrivateNumbers:
        pass

    class DHPublicKey:
        pass

    DHPublicKeyWithSerialization = DHPublicKey

    class DHPublicNumbers:
        pass

    generate_parameters = _nope("DH generate_parameters")


# --------------------------------------------------------------------- ciphers


class _CipherAlgorithmBase:
    pass


class AES:
    block_size = 128

    def __init__(self, key=None):
        self.key = key


AES128 = AES
AES256 = AES


class TripleDES:
    block_size = 64


class Blowfish:
    block_size = 64


class CAST5:
    block_size = 64


class Camellia:
    block_size = 128


class ChaCha20:
    block_size = 64


class IDEA:
    block_size = 64


class SEED:
    block_size = 128


class SM4:
    block_size = 128


class ARC4:
    block_size = 8


class RC2:
    block_size = 64


class ciphers_algorithms:
    AES = AES
    AES128 = AES128
    AES256 = AES256
    TripleDES = TripleDES
    Blowfish = Blowfish
    CAST5 = CAST5
    Camellia = Camellia
    ChaCha20 = ChaCha20
    IDEA = IDEA
    SEED = SEED
    SM4 = SM4
    ARC4 = ARC4
    BlockCipherAlgorithm = _CipherAlgorithmBase
    CipherAlgorithm = _CipherAlgorithmBase


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


class ciphers_modes:
    CBC = CBC
    CFB = CFB
    CFB8 = CFB8
    CTR = CTR
    ECB = ECB
    GCM = GCM
    OFB = OFB
    XTS = XTS
    Mode = Mode
    ModeWithAuthenticationTag = ModeWithAuthenticationTag
    ModeWithInitializationVector = ModeWithInitializationVector
    ModeWithNonce = ModeWithNonce
    ModeWithTweak = ModeWithTweak
    BlockCipherAlgorithm = BlockCipherAlgorithm
    CipherAlgorithm = _CipherAlgorithmBase
    UnsupportedAlgorithm = UnsupportedAlgorithm


class Cipher:
    """Construction is inert; encryptor/decryptor raise."""

    def __init__(self, algorithm=None, mode=None, backend=None):
        self.algorithm = algorithm
        self.mode = mode

    def encryptor(self):
        raise NotImplementedError("antipythony cryptography shim: encryption")

    def decryptor(self):
        raise NotImplementedError("antipythony cryptography shim: decryption")


class ciphers:
    Cipher = Cipher
    algorithms = ciphers_algorithms
    modes = ciphers_modes





# --- names re-exported by cryptography.hazmat.** leaf modules ---
_HashAlgorithm = HashAlgorithm  # legacy internal alias
class _AsymUtilsShim:
    Prehashed = Prehashed
    decode_dss_signature = decode_dss_signature
    encode_dss_signature = encode_dss_signature
    decode_rfc6979_signature = decode_rfc6979_signature
    encode_rfc6979_signature = encode_rfc6979_signature

_asym_utils_shim = _AsymUtilsShim()
