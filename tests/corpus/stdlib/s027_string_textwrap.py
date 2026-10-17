# Cites: S-05-4 (rows string, textwrap)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import string
import textwrap
names_s = ["ascii_letters", "ascii_lowercase", "ascii_uppercase", "digits",
           "hexdigits", "octdigits", "printable", "punctuation",
           "whitespace", "capwords", "Formatter", "Template"]
print(len(names_s), all(hasattr(string, n) for n in names_s))
print(string.digits, string.hexdigits, string.octdigits,
      string.ascii_uppercase[:3], len(string.ascii_letters),
      len(string.punctuation), repr(string.whitespace))
print(string.capwords("  a b  c "))
t = string.Template("$who likes $what")
print(t.substitute(who="x", what="y"))
print(t.safe_substitute(who="x"))
try:
    t.substitute(who="x")
except KeyError as e:
    print("template KeyError:", e)
print(string.Template("$$esc").safe_substitute())
print(string.Formatter().format("{0}-{1[0]}-{a.x}", 9, [8],
                               a=type("A", (), {"x": 7})()))
print(len(string.printable), repr(string.ascii_lowercase[:6]))
names_t = ["TextWrapper", "wrap", "fill", "shorten", "indent", "dedent"]
print(len(names_t), all(hasattr(textwrap, n) for n in names_t))
print(textwrap.wrap("the quick brown fox jumps", width=12))
print(textwrap.fill("the quick brown fox jumps", width=12).replace("\n", "|"))
print(repr(textwrap.shorten("a b c d e f", width=7)))
print(textwrap.indent("a\nb", "  ").replace("\n", "|"), "END")
print(repr(textwrap.dedent("    a\n      b\n")))
tw = textwrap.TextWrapper(width=10, break_long_words=False)
print(tw.wrap("aaaa bbbbccccdddd"))
print(textwrap.dedent("\nshared\nindent\n").strip().split())
