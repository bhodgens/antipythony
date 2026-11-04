"""antipythony deviation shim — OpenSSL.crypto (v0: constant surfaces)."""

FILETYPE_PEM = 1
FILETYPE_ASN1 = 2
FILETYPE_TEXT = 3


class X509:
    def get_subject(self):
        return self

    def get_extension_count(self):
        return 0


class PKey:
    pass


class X509Req:
    pass


def load_certificate(*a):
    return X509()


def load_privatekey(*a):
    return PKey()
