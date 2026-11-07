"""_multibytecodec: mixin classes over the native engine bridges.

NOTE (issue #13): the native module carries interned types whose instances
route encode/decode through the engine via the `codec` class attribute the
wrappers set (crates/ap-run/src/mbc.rs mbc_incremental_method). This vendored
file only exists to satisfy the augment hook; the mixin surface is native.
"""
print("")
