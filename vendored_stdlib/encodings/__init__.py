"""Minimal encodings package shim (s037): the codec registry's lookup path
needs `encodings` importable and `encodings.aliases.aliases` as the alias
table. Antipythony's codec core lives in the native `_codecs` surface; this
package supplies the alias mapping and the search-path hook shape.

W113: `normalize_encoding` ported verbatim from CPython 3.14 — w3lib's
`_c18n_encoding` (via scrapy's resolve_encoding chain for response.text)
calls it, and the aliases table is now the full CPython table so
latin_1/gb_2312_80/etc. canonicalize identically to CPython.
"""


def normalize_encoding(encoding):

    """ Normalize an encoding name.

        Normalization works as follows: all non-alphanumeric
        characters except the dot used for Python package names are
        collapsed and replaced with a single underscore, e.g. '  -;#'
        becomes '_'. Leading and trailing underscores are removed.

        Note that encoding names should be ASCII only.

    """
    if isinstance(encoding, bytes):
        encoding = str(encoding, "ascii")

    chars = []
    punct = False
    for c in encoding:
        if c.isalnum() or c == '.':
            if punct and chars:
                chars.append('_')
            if c.isascii():
                chars.append(c)
            punct = False
        else:
            punct = True
    return ''.join(chars)


def search_function(encoding):
    norm = normalize_encoding(encoding).lower()
    import encodings.aliases as _al
    name = _al.aliases.get(norm, norm)
    try:
        mod = __import__("encodings." + name, globals(), locals(), ["getregentry"])
    except Exception:
        return None
    return mod.getregentry()


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
