"""Minimal encodings package shim (s037): the codec registry's lookup path
needs `encodings` importable and `encodings.aliases.aliases` as the alias
table. Antipythany's codec core lives in the native `_codecs` surface; this
package only supplies the alias mapping and the search-path hook shape.
"""

_alias_table = {
    "latin_1": "latin_1",
    "latin1": "latin_1",
    "iso_8859_1": "latin_1",
    "iso8859_1": "latin_1",
    "ascii": "ascii",
    "us_ascii": "ascii",
    "utf_16": "utf_16",
    "u16": "utf_16",
    "utf_32": "utf_32",
    "u32": "utf_32",}


def search_function(encoding):
    norm = encoding.replace(" ", "_").replace("-", "_").lower()
    name = _alias_table.get(norm) or _alias_table.get(encoding)
    if name is None:
        return None
    import encodings.aliases  # noqa: F401
    return None  # v0: native _codecs registry answers before this hook


# CPython registers this via codecs.register at import.
import codecs as _codecs_mod

try:
    _codecs_mod.register(search_function)
except Exception:
    pass

# Expose the submodule (CPython: `import encodings.aliases` binds the
# package attr; our importer does that too, but bind explicitly so
# `encodings.aliases.aliases` resolves even when only `encodings` was
# imported first).
import encodings.aliases as _aliases_mod  # noqa: F401
