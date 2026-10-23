# Vendored string extensions (antipythany s027 fix pass).
#
# Template/Formatter are pure Python in CPython 3.14 (Lib/string.py,
# PSF-2.0); a hand-rolled scanner replaces the CPython regex engine here —
# the interp's `re` surface is a literal-prefix subset that can't handle
# the named-group alternation Template's default pattern needs.
#
# The native module (builtins_modules build_string) registers interned
# types for Template/Formatter; the native registration must be dropped so
# augment_with_vendored fills these names (see s027 note in Rust).

def _ap_raise_keyerror(key_repr):
    # Forward to the native dispatcher (string._ap_raise_keyerror) so the
    # KeyError renders with the args repr; fall back to a direct raise when
    # the native surface is absent (standalone runs).
    try:
        import string as _string

        native = getattr(_string, '_ap_raise_keyerror', None)
        if native is not None:
            native(key_repr)
    except KeyError:
        raise
    raise KeyError(key_repr)


class Template:
    """A string class for supporting $-substitutions."""

    delimiter = '$'
    idpattern = r'[_a-z][_a-z0-9]*'

    def __init__(self, template):
        self.template = template

    def _scan(self, s):
        """Yield (literal, is_dollar, name_or_None) tokens.

        CPython semantics: $$ -> literal $; $name / ${name} -> field;
        a lone $ at end / before a non-identifier char is an error
        (strict) or passed through (safe).
        """
        i = 0
        n = len(s)
        lit = []
        while i < n:
            c = s[i]
            if c != '$':
                lit.append(c)
                i += 1
                continue
            # c == '$'
            if i + 1 >= n:
                yield (''.join(lit), False, None, 'invalid')
                return
            nxt = s[i + 1]
            if nxt == '$':
                lit.append('$')
                i += 2
                continue
            if nxt == '{':
                j = s.find('}', i + 2)
                if j < 0:
                    yield (''.join(lit), False, None, 'invalid')
                    return
                name = s[i + 2:j]
                yield (''.join(lit), False, name, None)
                lit = []
                i = j + 1
                continue
            # $name
            j = i + 1
            if not (j < n and (s[j] == '_' or s[j].isalpha())):
                yield (''.join(lit), False, None, 'invalid')
                return
            k = j
            while k < n and (s[k] == '_' or s[k].isalpha() or s[k].isdigit()):
                k += 1
            yield (''.join(lit), False, s[j:k], None)
            lit = []
            i = k
        yield (''.join(lit), False, None, None)

    def _substitute(self, mapping, safe):
        out = []
        for lit, _d, name, err in self._scan(self.template):
            out.append(lit)
            if err:
                if not safe:
                    raise ValueError(
                        'Invalid placeholder in string: line 1, col 1')
                continue
            if name is None:
                continue
            if name in mapping:
                out.append(str(mapping[name]))
            elif safe:
                out.append(self.delimiter + name)
            else:
                # Route through the native raise so the KeyError renders with
                # the args repr (CPython: str(e) == repr(key)).
                _ap_raise_keyerror(name)
        return ''.join(out)

    def substitute(self, mapping=None, /, **kwds):
        if mapping is None:
            mapping = kwds
        elif kwds:
            merged = dict(mapping)
            merged.update(kwds)
            mapping = merged
        return self._substitute(mapping, safe=False)

    def safe_substitute(self, mapping=None, /, **kwds):
        if mapping is None:
            mapping = kwds
        elif kwds:
            merged = dict(mapping)
            merged.update(kwds)
            mapping = merged
        return self._substitute(mapping, safe=True)

    def __repr__(self):
        return 'string.Template(%r)' % (self.template,)


class Formatter:
    """format() over a mini field scanner (v0 subset: {idx}, {name},
    {idx[item]}, {name.attr})."""

    def _resolve(self, field_name, args, kwargs):
        # field_name = key[.attr][[item]]... (single level supported)
        base, sep, rest = field_name.partition('.')
        base, bsep, bitem = base.partition('[')
        if base.isdigit():
            val = args[int(base)]
        else:
            val = kwargs[base]
        if bsep:
            idx = bitem.rstrip(']')
            val = val[int(idx)] if idx.isdigit() else val[idx]
        if sep:
            val = getattr(val, rest.split('.')[0])
        return val

    def format(self, format_string, /, *args, **kwargs):
        out = []
        i = 0
        n = len(format_string)
        while i < n:
            c = format_string[i]
            if c == '}':
                if i + 1 < n and format_string[i + 1] == '}':
                    out.append('}')
                    i += 2
                    continue
                raise ValueError("Single '}' encountered in format string")
            if c != '{':
                out.append(c)
                i += 1
                continue
            if i + 1 < n and format_string[i + 1] == '{':
                out.append('{')
                i += 2
                continue
            j = format_string.find('}', i + 1)
            if j < 0:
                raise ValueError("Single '{' encountered in format string")
            field = format_string[i + 1:j]
            spec = ''
            if ':' in field:
                field, spec = field.split(':', 1)
            conv = ''
            if '!' in field:
                field, conv = field.rsplit('!', 1)
            val = self._resolve(field, args, kwargs)
            if conv == 'r':
                out.append(repr(val))
            elif conv == 's':
                out.append(str(val))
            else:
                out.append(format(val, spec) if spec else str(val))
            i = j + 1
        return ''.join(out)

    def vformat(self, format_string, args, kwargs):
        return self.format(format_string, *args, **kwargs)

    def get_field(self, field_name, args, kwargs):
        return (self._resolve(field_name, args, kwargs), field_name)
