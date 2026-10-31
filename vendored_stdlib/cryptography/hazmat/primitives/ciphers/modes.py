"""antipythany deviation shim — cryptography.hazmat.primitives.ciphers.modes (v0)."""
from cryptography import ciphers as _c

CBC = _c.modes.CBC
CFB = _c.modes.CFB
CFB8 = _c.modes.CFB8
CTR = _c.modes.CTR
ECB = _c.modes.ECB
GCM = _c.modes.GCM
OFB = _c.modes.OFB
XTS = _c.modes.XTS
Mode = _c.modes.Mode
ModeWithAuthenticationTag = _c.modes.ModeWithAuthenticationTag
ModeWithInitializationVector = _c.modes.ModeWithInitializationVector
ModeWithNonce = _c.modes.ModeWithNonce
ModeWithTweak = _c.modes.ModeWithTweak
BlockCipherAlgorithm = _c.modes.BlockCipherAlgorithm
CipherAlgorithm = _c.modes.CipherAlgorithm
