# Vendored string error helpers (antipythony s027 fix pass).
#
# Template.substitute raises KeyError for a missing placeholder. When the
# raise happens inside vendored Python code the interp's exception object
# str() path renders the default object repr instead of the args repr;
# raising through this native-routed module fn produces a properly rendered
# KeyError (args repr), matching CPython's `str(e) == repr(key)`.

__all__ = ['raise_keyerror']


def raise_keyerror(key_repr):
    """Raise KeyError(key) — the caller passes repr(key) pre-formatted."""
    raise KeyError(key_repr)
