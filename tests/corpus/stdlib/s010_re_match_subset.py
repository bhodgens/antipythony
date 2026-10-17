# Cites: S-05-4 (row re), S-05-22, anchor #6
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import re
names = ["compile", "match", "fullmatch", "search", "findall", "finditer",
         "sub", "subn", "split", "purge", "escape", "A", "I", "M", "S", "X",
         "L", "U", "DOTALL", "IGNORECASE", "LOCALE", "MULTILINE", "NOFLAG",
         "VERBOSE", "ASCII", "UNICODE", "Pattern", "Match", "RegexFlag",
         "PatternError", "error", "N"]
print(len(names), all(hasattr(re, n) for n in names))
print(int(re.A), int(re.I), int(re.M), int(re.S), int(re.X), int(re.L),
      int(re.U), re.DOTALL is re.S, re.IGNORECASE is re.I, re.MULTILINE is re.M,
      re.VERBOSE is re.X, re.ASCII is re.A, re.NOFLAG)
print(re.error is re.PatternError)
print(re.findall(r"\d+", "a1b22"), re.sub(r"\s+", "-", "x  y"))
p = re.compile(r"(?P<w>\w+)@(?P<d>[\w.]+)")
m = p.search("mail: al@ex.com.pl end")
print(m.group(), m.group(1), m.group("d"), m.groups())
print(m.span(), m.start(), m.end(), m.lastgroup, m.lastindex)
print(bool(re.match("a+", "aaab")), bool(re.fullmatch("a+", "aaab")),
      bool(re.fullmatch("a+", "aaabc")))
print(re.split(r",\s*", "a, b,c"), re.split(r"\W", "a1b", maxsplit=2))
print(re.findall(r"[a-z]+", "Ab1cD"))
print(re.findall(r"[a-z]+", "Ab1cD", re.I))
print(re.sub(r"(\d)", lambda mt: f"<{mt[0]}>", "a1b2"))
print(re.subn(r"o", "0", "foo boo"))
print([mt.group() for mt in re.finditer(r"\w+", "hi there 42")])
print(re.escape("a.b*c"), re.sub(re.escape("a.b"), "X", "xa.by"))
print(p.pattern, p.flags, p.groups, p.groupindex)
mm = re.match(r"(a)(?:(b)|(c))", "ac")
print(mm.group(2), mm[3], mm.string == "ac")
print(type(p).__name__, type(mm).__name__, isinstance(mm, re.Match),
      isinstance(p, re.Pattern))
print(re.compile("x", re.X).search("  x").group(0))
print(" ".join(sorted(re.findall(r"\w", "cba"))))
print(re.sub("a+", "-", "aaa bbb", flags=re.IGNORECASE) if False else
      re.sub("A+", "-", "aaa bbb", flags=re.IGNORECASE))
