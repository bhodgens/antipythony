# antipythony shim — service_identity (twisted _sslverify chain; v0 constant/method surfaces only)
def extract_from_certificate(*a, **kw):
    return []

def extract_from_ssl_socket(*a, **kw):
    return []

def extract_patterns(*a, **kw):
    return []

def verify_hostname(*a, **kw):
    return None

def verify_ip_address(*a, **kw):
    return None

class VerificationError(Exception):
    pass

class CertificateError(Exception):
    pass
