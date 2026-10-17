# Cites: S-05-4 (rows token, tokenize)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import io
import token
import tokenize
names_t = ["tok_name", "EXACT_TOKEN_TYPES", "ISEOF", "ISNONTERMINAL",
           "ISTERMINAL"]
print(all(hasattr(token, n) for n in names_t), token.tok_name[token.LPAR],
      token.EXACT_TOKEN_TYPES["+"], token.tok_name[token.COMMENT],
      token.tok_name[token.NEWLINE])
print(token.ISTERMINAL(token.NEWLINE), token.ISNONTERMINAL(token.NAME),
      token.ISEOF(token.ENDMARKER))
names_k = ["generate_tokens", "Token", "TokenInfo", "untokenize",
           "detect_encoding", "open", "TokenError", "tabsize", "COMMENT",
           "NL"]
print(all(hasattr(tokenize, n) for n in names_k), tokenize.tabsize,
      tokenize.COMMENT, tokenize.NL, tokenize.ENCODING)
src = "def f(x):\n    return x + 1\n"
toks = [(token.tok_name[t.type], t.string) for t in
        tokenize.generate_tokens(io.StringIO(src).readline)
        if t.type not in (token.NEWLINE, token.ENDMARKER)]
print(toks)
print(tokenize.TokenInfo._fields)
print(repr(tokenize.untokenize([(token.NAME, "abc"), (token.NEWLINE, "\n")])))
print(tokenize.detect_encoding(
      io.BytesIO(b"# coding: utf-8\nx=1\n").readline))
try:
    tokenize.untokenize("not-tokens")
except Exception as e:
    print("untokenize-bad:", type(e).__name__,
          isinstance(e, tokenize.TokenError))
print(issubclass(tokenize.TokenError, Exception))
