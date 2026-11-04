"""antipythony deviation shim — ssl (v0: the file:// gate imports ssl for
constant surfaces; no TLS is performed)."""


class SSLError(OSError):
    pass


class SSLCertVerificationError(SSLError):
    pass


SSLWantReadError = SSLError
SSLWantWriteError = SSLError
SSLSyscallError = SSLError
SSLEOFError = SSLError
SSLZeroReturnError = SSLError

CERT_NONE = 0
CERT_OPTIONAL = 1
CERT_REQUIRED = 2

PROTOCOL_TLS = 771

class TLSVersion:
    SSLv3 = "SSLv3"
    TLSv1 = "TLSv1"
    TLSv1_1 = "TLSv1_1"
    TLSv1_2 = "TLSv1_2"
    TLSv1_3 = "TLSv1_3"
    MINIMUM_SUPPORTED = "MIN"
    MAXIMUM_SUPPORTED = "MAX"
PROTOCOL_TLS_CLIENT = PROTOCOL_TLS
PROTOCOL_TLS_SERVER = PROTOCOL_TLS
PROTOCOL_SSLv23 = PROTOCOL_TLS

OP_NO_SSLv2 = 0x1000000
OP_NO_SSLv3 = 0x2000000
OP_NO_TLSv1 = 0x4000000
OP_NO_TLSv1_1 = 0x10000000
OP_NO_TLSv1_2 = 0x8000000
OP_NO_TLSv1_3 = 0x20000000
OP_NO_COMPRESSION = 0x20000
OP_CIPHER_SERVER_PREFERENCE = 0x400000
OP_SINGLE_DH_USE = 0x80000
OP_SINGLE_ECDH_USE = 0x80000
OP_ENABLE_MIDDLEBOX_COMPAT = 0x1000000

VERIFY_NONE = CERT_NONE
VERIFY_PEER = CERT_REQUIRED

HAS_SNI = True
HAS_TLSv1_2 = True
HAS_TLSv1_3 = True

OPENSSL_VERSION = "OpenSSL 3.0.0 (antipythony shim)"
OPENSSL_VERSION_NUMBER = 0x30000000
OPENSSL_VERSION_INFO = (3, 0, 0, 0, 0)


class SSLContext:
    def __init__(self, protocol=PROTOCOL_TLS_CLIENT):
        self.protocol = protocol
        self.verify_mode = CERT_NONE
        self.check_hostname = False

    def load_cert_chain(self, *a, **k):
        pass

    def load_verify_locations(self, *a, **k):
        pass

    def set_ciphers(self, ciphers):
        pass

    def wrap_socket(self, sock, *a, **k):
        return sock

    def wrap_bio(self, *a, **k):
        raise NotImplementedError("ssl shim: wrap_bio unsupported in v0")


def create_default_context(purpose=None, *, cafile=None, capath=None,
                           cadata=None):
    return SSLContext(PROTOCOL_TLS_CLIENT)


def _create_unverified_context(*a, **k):
    return SSLContext(PROTOCOL_TLS_CLIENT)


create_urllib3_context = _create_unverified_context
SSLContext_num = SSLContext


def get_server_certificate(addr, ssl_version=None, ca_certs=None):
    return ""


def PEM_cert_to_DER_cert(pem):
    return pem.encode()


def DER_cert_to_PEM_cert(der):
    return der


def rand_bytes(n):
    import os

    return os.urandom(n)


DEFAULT_CIPHERS = "DEFAULT"
_alertDescriptions = {}
_errorCodes = {}
