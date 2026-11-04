# antipythony shim — service_identity (twisted _sslverify chain; v0 constant/method surfaces only)
class DNS_ID:
    def __init__(self, name):
        self.name = name
    def verify(self, *a, **kw):
        return None

class IPAddress_ID:
    def __init__(self, ip):
        self.ip = ip
    def verify(self, *a, **kw):
        return None

class ServiceID:
    pass

def verify_service_identity(*a, **kw):
    return None
