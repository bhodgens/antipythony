"""antipythony deviation shim — serialization classes (single source)."""


def _nope(msg):
    def f(*a, **k):
        raise NotImplementedError("antipythony cryptography shim: %s" % msg)

    return f


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
