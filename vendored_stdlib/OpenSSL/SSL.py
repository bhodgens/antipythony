"""antipythany deviation shim — OpenSSL.SSL (v0: constant surfaces only;
the file:// gate performs no TLS)."""


class Error(Exception):
    pass


TLS_METHOD = 771
TLS_CLIENT_METHOD = 771
TLS_SERVER_METHOD = 771
SSLv23_METHOD = 0x1000
TLSv1_2_METHOD = 0x1000
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
OP_ALL = 0
OP_NO_TICKET = 0x4000
OP_NO_QUERY_MTU = 0x1000
OP_COOKIE_EXCHANGE = 0x2000
OP_MICROSOFT_BIG_SSLV3_BUFFER = 0x20
OP_MICROSOFT_SESS_ID_BUG = 0x4
OP_NETSCAPE_CHALLENGE_BUG = 0x8
OP_NETSCAPE_REUSE_CIPHER_CHANGE_BUG = 0x10
OP_SSLREF2_REUSE_CERT_TYPE_BUG = 0x2
OP_TLS_BLOCK_PADDING_BUG = 0x40
OP_DONT_INSERT_EMPTY_FRAGMENTS = 0x800
OP_PASS_CLIENT_CERT = 0x4
VERIFY_NONE = 0
VERIFY_PEER = 2
VERIFY_FAIL_IF_NO_PEER_CERT = 1
VERIFY_CLIENT_ONCE = 4
CERT_NONE = 0
CERT_REQUIRED = 2
CERT_OPTIONAL = 1
SENT_SHUTDOWN = 1
RECEIVED_SHUTDOWN = 2
ERROR_WANT_READ = 2
ERROR_WANT_WRITE = 3
SESS_CACHE_OFF = 0
SESS_CACHE_SERVER = 1
SSL_ST_OK = 3
FILETYPE_PEM = 1


class Context:
    def __init__(self, method=None):
        pass

    def use_privatekey_file(self, *a):
        pass

    def use_certificate_file(self, *a):
        pass

    def set_cipher_list(self, *a):
        pass

    def set_options(self, *a):
        return 0

    def set_verify(self, *a):
        pass

    def check_privatekey(self):
        pass


class Connection:
    _app_data = None

    def __init__(self, context, sock=None):
        self._sock = sock

    def do_handshake(self):
        pass

    def send(self, data):
        return len(data)

    def recv(self, n):
        return b""

    def shutdown(self):
        return True

    def close(self):
        pass

    def get_cipher_list(self):
        return ["DEFAULT"]

    def get_servername(self):
        return None

    def set_servername(self, name):
        return None

    def get_app_data(self):
        return self._app_data

    def set_app_data(self, data):
        self._app_data = data
        return None

    def getpeercert(self):
        return None


WantReadError = Error
WantWriteError = Error
SysCallError = Error
ZeroReturnError = Error


# Wave-79: scrapy/utils/ssl.py probes the OpenSSL build via
# SSLeay_version(SSLEAY_VERSION). Real pyOpenSSL exposes the
# SSLeay_* constants (SSLEAY_VERSION = 0) and SSLeay_version()
# returning the OpenSSL banner string.
SSLEAY_VERSION = 0
SSLEAY_CFLAGS = 1
SSLEAY_BUILT_ON = 2
SSLEAY_PLATFORM = 3
SSLEAY_DIR = 4


def SSLeay_version(t: int = SSLEAY_VERSION) -> str:
    if t == SSLEAY_VERSION:
        return "OpenSSL 3.0.0 (antipythany shim)"
    return "antipythany shim"


def OPENSSL_version(*args: object) -> str:
    return SSLeay_version(*args)
