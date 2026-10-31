"""antipythany deviation shim — cryptography.hazmat.primitives.asymmetric.padding (v0)."""
from cryptography import padding as _p

AsymmetricPadding = _p.AsymmetricPadding
MGF = _p.MGF
MGF1 = _p.MGF1
OAEP = _p.OAEP
PKCS1v15 = _p.PKCS1v15
PSS = _p.PSS
calculate_max_pss_salt_length = _p.calculate_max_pss_salt_length
