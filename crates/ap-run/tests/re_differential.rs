// Leaf 02.08-full p2 — re engine differential probes vs live CPython 3.14.
// Each probe runs the same snippet through the reference interpreter and
// ap-run-eval and requires byte-exact stdout.
//
// Families: literals/classes/escapes, quantifiers (greedy/lazy), alternation,
// groups + backrefs + named groups, lookarounds, anchors, flags (I M S X A),
// match-object surface, module API (compile/finditer/findall/search/match/
// fullmatch/sub/subn/split/escape/purge), template repl semantics, error
// texts, Pattern/Match reprs.

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14, version-string checked; AP_REQUIRE_REFERENCE=1 turns a
// missing reference into a hard failure instead of a silent green.
mod common;

fn run_reference(py: &str, src: &str) -> Vec<u8> {
    let out = std::process::Command::new(py)
        .arg("-S")
        .arg("-c")
        .arg(src)
        .output()
        .expect("reference python");
    out.stdout
}

fn run_aprun(src: &str) -> Vec<u8> {
    // Shared helper: subprocess-per-probe via ap-run-probe, bin-fresh
    // (CARGO_BIN_EXE_ — replaces the old prebuilt-ap-run-eval caveat),
    // serialized by the heavy-gate flock (see tests/common/mod.rs).
    let (out, _err, _code) = common::run_aprun_eval(src);
    out
}

fn check(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let reference = run_reference(py, src);
    let ours = run_aprun(src);
    assert_eq!(
        reference,
        ours,
        "probe {name}: stdout mismatch\n ref: {}\n ours: {}",
        String::from_utf8_lossy(&reference),
        String::from_utf8_lossy(&ours)
    );
}

// ------------------------------------------------- literals/classes ---

#[test]
fn literals_dot_charset() {
    common::interpreter_gate();
    check(
        "literals-dot-charset",
        r#"
import re
print(re.match('abc', 'abcdef').group())
print(re.match('abc', 'xbc'))
print(re.findall('a.c', 'abc a.c a+c'))
print(re.findall('a[bc]d', 'abd acd aed'))
print(re.findall('[a-z]+', 'Ab1cD'))
print(re.findall('[^a-z]+', 'Ab1cD'))
print(re.findall('a[.]c', 'a.c abc'))
print(re.findall('[-a]+', 'x-a-b'))
print(re.findall('[a-]+', 'x-a-b'))
print(re.findall('[]a]', ']a'))
print(re.findall(r'[\d]', 'a1'))
print(re.search('x$y', 'x'))
"#,
    );
}

#[test]
fn escapes_categories() {
    common::interpreter_gate();
    check(
        "escapes-categories",
        r#"
import re
print(re.findall(r'\d+', 'a1 b22 c333'))
print(re.findall(r'\D+', 'a1 b22'))
print(re.findall(r'\w+', 'hi there_1!'))
print(re.findall(r'\W+', 'hi there_1!'))
print(re.findall(r'\s', 'a b\tc'))
print(re.findall(r'\S+', 'a b\tc'))
print(re.findall(r'[\w.]+', 'a.b c'))
print(re.findall(r'\x41', 'AAB'))
print(re.findall(r'\.', 'a.b'))
print(re.split(r'[\t ]+', 'a b\tc'))
"#,
    );
}

#[test]
fn unicode_categories() {
    common::interpreter_gate();
    check(
        "unicode-categories",
        r#"
import re
print(re.findall(r'\w', 'äöü'))
print(re.findall(r'\d', '١٢٣'))
print(re.findall(r'\w', 'ä', re.A))
print(re.findall(r'\d', '١٢٣', re.A))
"#,
    );
}

#[test]
fn anchors_bA() {
    common::interpreter_gate();
    check(
        "anchors",
        r#"
import re
print(re.findall(r'\bcat\b', 'cat catalog concat'))
print(re.findall(r'\Bx', 'x yx'))
print([m.span() for m in re.finditer(r'\A\w+', 'ab\ncd')])
print(re.findall(r'\w+\Z', 'ab\n', re.M))
print(re.match(r'\Aab', 'ab').span())
print([m.span() for m in re.finditer(r'^', 'a\nb')])
print([m.span() for m in re.finditer(r'$', 'a\nb')])
print([m.span() for m in re.finditer(r'^', 'a\nb', re.M)])
"#,
    );
}

#[test]
fn quantifiers_greedy_lazy() {
    common::interpreter_gate();
    check(
        "quantifiers",
        r#"
import re
print(re.findall(r'a*', 'baab'))
print([m.span() for m in re.finditer(r'a*', 'baab')])
print(re.findall(r'a+', 'baab'))
print(re.match(r'a?', 'aaa').group())
print(re.findall(r'a{2,3}', 'aaaa'))
print(re.findall(r'a{2}', 'aaaa'))
print(re.findall(r'a{2,}', 'aaaa'))
print(re.findall(r'<.*>', '<a><b>'))
print(re.findall(r'<.*?>', '<a><b>'))
print(re.findall(r'a+?', 'aaa'))
print(re.match(r'a{0}', 'b').span())
print(re.match(r'x{,2}y', 'y').group())
"#,
    );
}

#[test]
fn quantifiers_backtrack_order() {
    common::interpreter_gate();
    check(
        "quantifiers-backtrack",
        r#"
import re
print(re.match(r'a*ab', 'aaab').group())
print(re.match(r'(a*)*b', 'aaab').groups())
print(re.match(r'(a|ab)(c|bcd)', 'abcd').groups())
print(re.match(r'(a|ab)(c|bcd)(d*)', 'abcd').groups())
print(re.match(r'a{2,4}?a', 'aaaaa').span())
print(re.match(r'(a?)(a?)a', 'aaa').groups())
"#,
    );
}

#[test]
fn alternation_order() {
    common::interpreter_gate();
    check(
        "alternation",
        r#"
import re
print(re.match(r'a|ab', 'ab').group())
print(re.findall(r'cat|catalog', 'catalog'))
print(re.match(r'(?:a|b)+', 'abab').group())
print(re.split(r'a|b', 'xaybz'))
print(re.match(r'x(?:one|two)', 'xtwo').group())
"#,
    );
}

#[test]
fn groups_basic() {
    common::interpreter_gate();
    check(
        "groups-basic",
        r#"
import re
m = re.match(r'(\w+) (\w+)', 'hello world')
print(m.groups(), m.group(0), m.group(1, 2), m.groupdict())
m2 = re.match(r'(\w+)(\d*)', 'abc42')
print(m2.groups(), m2.lastindex)
m3 = re.match(r'(a)?(b)', 'b')
print(m3.groups(), m3.lastindex, m3.group(1))
"#,
    );
}

#[test]
fn groups_nested_optional() {
    common::interpreter_gate();
    check(
        "groups-nested",
        r#"
import re
m = re.match(r'(a)(?:(b)|(c))', 'ac')
print(m.groups(), m.lastindex, m.lastgroup)
m2 = re.match(r'(a)(?:(b)|(c))', 'ab')
print(m2.groups(), m2.lastindex, m2.lastgroup)
m3 = re.match(r'((a)b)?', 'ab')
print(m3.groups(), m3.span(1), m3.span(2))
m4 = re.match(r'(a(b(c)))', 'abc')
print(m4.groups())
"#,
    );
}

#[test]
fn named_groups() {
    common::interpreter_gate();
    check(
        "named-groups",
        r#"
import re
m = re.match(r'(?P<x>\w+)/(?P<y>\d+)', 'abc/42')
print(m.group('x'), m.group('y'), m.groupdict(), m.lastgroup)
print(m.expand(r'\g<y>/\g<x>'))
print(m.expand(r'\2/\1'))
print(re.sub(r'(?P<a>\d)(?P<b>\w)', r'\g<b>\g<a>', '1a'))
p = re.compile(r'(?P<q>\w+)\s*(?P=q)')
print(p.match('ab ab').group(), p.match('ab cd'))
"#,
    );
}

#[test]
fn backrefs() {
    common::interpreter_gate();
    check(
        "backrefs",
        r#"
import re
print(re.match(r'(a)\1', 'aa').groups())
print(re.match(r'(a)\1', 'ab'))
print(re.match(r'(\w+) \1', 'hi hi').group())
print(re.match(r'(\w+) \1', 'hi ho'))
m = re.match(r'(a)(b)?\2', 'ab')
print(m.groups() if m else None)
m2 = re.match(r'(a)(b)?\2', 'a')
print(m2.groups() if m2 else None)
"#,
    );
}

#[test]
fn conditional_groups() {
    common::interpreter_gate();
    check(
        "conditional",
        r#"
import re
print(re.match(r'(a)?b(?(1)c|d)', 'abc').group())
print(re.match(r'(a)?b(?(1)c|d)', 'abd').group())
print(re.match(r'(a)?b(?(1)c|d)', 'acd'))
print(re.match(r'(a)?b(?(1)c)', 'ab').group())
"#,
    );
}

#[test]
fn lookahead() {
    common::interpreter_gate();
    check(
        "lookahead",
        r#"
import re
print(re.findall(r'\w+(?=@)', 'a@ b@ c'))
print(re.search(r'foo(?=bar)', 'foobar').span(), re.search(r'foo(?=bar)', 'foobaz'))
print(re.findall(r'\d+(?!x)', '1x 2y'))
print([m.span() for m in re.finditer(r'a(?=b)', 'ab ac ab')])
m = re.match(r'(?=(a+))a', 'aaa')
print(m.group(1), m.group())
"#,
    );
}

#[test]
fn lookbehind() {
    common::interpreter_gate();
    check(
        "lookbehind",
        r#"
import re
print(re.findall(r'(?<=@)\w+', '@a @b c'))
print(re.findall(r'(?<!x)a', 'xa aa ba'))
print([m.span() for m in re.finditer(r'(?<=-)x', 'a-x b-x y')])
print(re.match(r'(?<=a)b', 'ab').span())
"#,
    );
}

#[test]
fn lookbehind_variable_width_error() {
    common::interpreter_gate();
    check(
        "lookbehind-varwidth",
        r#"
import re
try:
    re.compile(r'(?<=a+)b')
except Exception as e:
    print(type(e).__name__, e)
try:
    re.compile(r'(?<!a*)b')
except Exception as e:
    print(type(e).__name__, e)
"#,
    );
}

#[test]
fn atomic_possessive() {
    common::interpreter_gate();
    check(
        "atomic-possessive",
        r#"
import re
print(re.match(r'(?>a*)a', 'aaa'))
print(re.match(r'a*+a', 'aaa'))
print(re.match(r'(?>a*)b', 'aaab').group())
print(re.match(r'a++b', 'aab').group())
"#,
    );
}

#[test]
fn flag_ignorecase() {
    common::interpreter_gate();
    check(
        "flag-i",
        r#"
import re
print(re.findall(r'[a-z]+', 'Ab1cD', re.I))
print(re.match(r'hello', 'HELLO', re.I).group())
print(re.sub(r'A+', '-', 'aaa', flags=re.I))
print(re.fullmatch(r'ab', 'AB', re.I).group())
print(re.compile('x', re.I).pattern)
"#,
    );
}

#[test]
fn flag_multiline_dotall() {
    common::interpreter_gate();
    check(
        "flag-ms",
        r#"
import re
print(re.findall(r'^\w+', 'ab\ncd', re.M))
print(re.findall(r'\w+$', 'ab\ncd', re.M))
print(re.match('a.b', 'a\nb', re.S).group())
print(re.match('a.b', 'a\nb'))
print(re.findall(r'(?m)^\d', '1a\n2b'))
print(re.findall(r'(?s)a.b', 'axb'))
"#,
    );
}

#[test]
fn flag_verbose() {
    common::interpreter_gate();
    check(
        "flag-x",
        r#"
import re
print(re.match(r'a b', 'ab', re.X).group())
print(re.match(r'a#b', 'a', re.X).group())
p = re.compile(r'(\d+)\s+(\w+)', re.X)
print(p.match('42 koala').groups())
"#,
    );
}

#[test]
fn flag_ascii_inline() {
    common::interpreter_gate();
    check(
        "flag-inline",
        r#"
import re
print(re.findall(r'(?i)ab', 'aB'))
print(re.findall(r'(?i:a)b', 'AB aB'))
print(re.findall(r'(?-i:a)b', 'ab', re.I))
print(re.findall(r'\w+', 'ä', re.A))
"#,
    );
}

#[test]
fn match_object_surface() {
    common::interpreter_gate();
    check(
        "match-object",
        r#"
import re
m = re.match(r'(\w+)@(\w+)', 'user host')
print(m.group(), m.start(), m.end(), m.span())
print(m.span(1), m.start(2), m.end(0))
print(m.re.pattern, m.string, m.pos, m.endpos)
print(m.regs)
print(m[0], m[1])
print(m.expand(r'\1!'))
print(repr(m))
print(m.lastindex, m.lastgroup)
"#,
    );
}

#[test]
fn match_repr_edge() {
    common::interpreter_gate();
    check(
        "match-repr",
        r#"
import re
m = re.match(r'a', 'a')
print(repr(m))
m2 = re.match(r'', 'ab')
print(repr(m2))
m3 = re.match(r"(a)'b", "a'b")
print(repr(m3))
"#,
    );
}

#[test]
fn module_compile_api() {
    common::interpreter_gate();
    check(
        "module-compile",
        r#"
import re
p = re.compile(r'\d+')
print(p.match('a42b').group(), p.search('a42b').group(), p.fullmatch('a42b'))
print(p.findall('a1b2'), [m.group() for m in p.finditer('a1b2')])
print(p.split('a1b22c'), p.split('a1b22c', 1))
print(p.sub('#', 'a1b2'), p.subn('#', 'a1b2', 1))
print(p.pattern, p.flags, p.groups, p.groupindex)
print(bool(re.compile('')), re.compile('ab', 0).pattern)
"#,
    );
}

#[test]
fn module_search_match_fullmatch() {
    common::interpreter_gate();
    check(
        "module-anchors",
        r#"
import re
print(re.match('a+', 'aaab').group())
print(re.fullmatch('a+', 'aaab').group())
print(re.fullmatch('a+', 'aaabc'))
print(re.search('b+', 'aaabbc').group())
print(re.match('b+', 'aaab'))
print(re.search('^b', 'ab', re.M).span())
"#,
    );
}

#[test]
fn sub_repl_forms() {
    common::interpreter_gate();
    check(
        "sub-repl",
        r#"
import re
print(re.sub(r'(\d)', r'<\1>', 'a1b2'))
print(re.sub(r'(?P<n>\d)', r'[\g<n>]', 'a1'))
print(re.sub(r'a', 'x', 'aaa', 1))
print(re.sub(r'x*', '-', 'abxcd'))
print(re.sub(r'', '-', 'ab'))
print(re.subn(r'o', '0', 'foo boo'))
print(re.sub(r'(a)|(b)', lambda m: '1' if m.group(1) else '2', 'abab'))
print(re.sub(r'a', r'\\', 'aa'))
"#,
    );
}

#[test]
fn split_semantics() {
    common::interpreter_gate();
    check(
        "split",
        r#"
import re
print(re.split(r',\s*', 'a, b,c'))
print(re.split(r'\W', 'a1b', maxsplit=2))
print(re.split(r'(x)', 'axbxc'))
print(re.split(r'x*', 'abxcd'))
print(re.split(r'', 'ab'))
"#,
    );
}

#[test]
fn findall_group_forms() {
    common::interpreter_gate();
    check(
        "findall-groups",
        r#"
import re
print(re.findall(r'(a)(b)?', 'ab a'))
print(re.findall(r'(a)', 'aba'))
print(re.findall(r'a', 'aba'))
print(re.findall(r'(a)(b)(c)', 'abc'))
"#,
    );
}

#[test]
fn escape_fn() {
    common::interpreter_gate();
    check(
        "escape",
        r#"
import re
print(re.escape('a.b*c'))
print(re.escape('a1_'))
print(re.sub(re.escape('a.b'), 'X', 'xa.by'))
print(re.escape('()[]{}?*+-|^$\\.&~# \t\n'))
"#,
    );
}

#[test]
fn purge_and_cache() {
    common::interpreter_gate();
    check(
        "purge",
        r#"
import re
print(re.purge() is None)
p1 = re.compile('abc')
re.purge()
p2 = re.compile('abc')
print(p1.pattern == p2.pattern)
"#,
    );
}

#[test]
fn types_and_names() {
    common::interpreter_gate();
    check(
        "types-names",
        r#"
import re
p = re.compile('x')
m = p.match('x')
print(type(p).__name__, type(m).__name__)
print(isinstance(m, re.Match), isinstance(p, re.Pattern))
print(int(re.A), int(re.I), int(re.M), int(re.S), int(re.X), int(re.L), int(re.U))
print(re.DOTALL is re.S, re.IGNORECASE is re.I, re.MULTILINE is re.M,
      re.VERBOSE is re.X, re.ASCII is re.A)
print(re.error is re.PatternError)
print(issubclass(re.error, Exception))
"#,
    );
}

#[test]
fn error_texts() {
    common::interpreter_gate();
    check(
        "errors",
        r#"
import re
def show(fn, *a):
    try:
        v = fn(*a)
        return 'value'
    except Exception as e:
        return f'{type(e).__name__}: {e}'
print(show(re.compile, '('))
print(show(re.compile, '['))
print(show(re.compile, '*'))
print(show(re.compile, 'a**'))
print(show(re.compile, '(?P<n>a)(?P<n>b)'))
print(show(re.sub, '(a)', 'x\\1\\2', 'a'))
print(show(re.compile, 'a{2,1}'))
print(show(re.compile, '\\q'))
print(show(re.compile, '(a'))
print(show(re.compile, 'a)'))
"#,
    );
}

#[test]
fn zero_width_semantics() {
    common::interpreter_gate();
    check(
        "zero-width",
        r#"
import re
print([m.span() for m in re.finditer(r'', 'ab')])
print([m.span() for m in re.finditer(r'a*', 'baab')])
print(re.sub(r'', '-', 'abc'))
print([m.span() for m in re.finditer(r'\b', 'ab cd')])
print(re.match(r'(a?)*b', 'b').groups())
"#,
    );
}

#[test]
fn pos_endpos() {
    common::interpreter_gate();
    check(
        "pos-endpos",
        r#"
import re
p = re.compile('ab')
print(p.match('abab', 2).group())
print(p.match('abab', 2, 4).span())
print(p.search('abab', 1).span())
print(p.fullmatch('xxab', 2).group())
print(p.match('ab', 5))
"#,
    );
}

#[test]
fn warnings_style_patterns() {
    common::interpreter_gate();
    check(
        "warnings-patterns",
        r#"
import re
p = re.compile('DeprecationWarning', re.I)
print(p.match('DeprecationWarning').group(), p.match('deprecationwarning').group())
m = re.compile(r'\AWarning\Z').match('Warning')
print(bool(m))
print(re.escape('module_name') + r'\z')
print(bool(re.match(r'(-*A-*)', 'A')))
print(re.match(r'(-*A-*)', '-A-').group(1))
print(bool(re.match(r'(-*A?-*)', '')))
print(bool(re.match(r'(-*[A-]*)', 'AO')))
"#,
    );
}

#[test]
fn pattern_object_methods() {
    common::interpreter_gate();
    check(
        "pattern-methods",
        r#"
import re
p = re.compile(r'(\w)(\d)')
print(p.sub(r'\2\1', 'a1b2'))
print(p.subn(r'<\1\2>', 'a1', 3))
m = p.search('x9y8')
print(m.groups(), m.span(2))
print(p.findall('a1b2'))
print(p.split('a1b2'))
"#,
    );
}

#[test]
fn misc_api() {
    common::interpreter_gate();
    check(
        "misc-api",
        r#"
import re
print(hasattr(re, 'Scanner'))
print(hasattr(re, 'purge'), hasattr(re, 'escape'))
"#,
    );
}

#[test]
fn nested_quantifier_groups() {
    common::interpreter_gate();
    check(
        "nested-quant",
        r#"
import re
print(re.match(r'(a*)*b', 'aaab').group())
print(re.match(r'(a*)*b', 'aaab').groups())
print(re.match(r'(|a)*b', 'ab').groups())
print(re.match(r'(a|)*b', 'aab').groups())
print(re.findall(r'(a|b)+', 'abab'))
print(re.match(r'((a)|b)+', 'aba').groups())
"#,
    );
}

#[test]
fn casefold_edge() {
    common::interpreter_gate();
    check(
        "casefold",
        r#"
import re
print(re.match(r'STRASSE', 'strasse', re.I) is not None)
print(re.match(r'ſ', 's', re.I) is not None)
print(re.match(r'k', 'K', re.I).group())
print(re.findall(r'[a-z]+', 'ABC', re.I))
print(re.match(r'[A-Z]', 'a', re.I).group())
"#,
    );
}

#[test]
fn multiline_dollar() {
    common::interpreter_gate();
    check(
        "multiline-dollar",
        r#"
import re
print([m.span() for m in re.finditer(r'x$', 'ax\nbx')])
print([m.span() for m in re.finditer(r'x$', 'ax\nbx', re.M)])
print(re.match(r'a$', 'a\n').span())
print(re.match(r'a$', 'a\nb'))
print(re.findall(r'^', 'a\nb\nc', re.M))
"#,
    );
}

#[test]
fn groupref_in_repeat() {
    common::interpreter_gate();
    check(
        "groupref-repeat",
        r#"
import re
print(re.match(r'(\w+) \1', 'hey hey').group())
print(re.match(r'(?:(\w) \1)+', 'a a b b').group())
print(re.match(r'(a|b)\1+', 'abbb').group())
print(re.match(r'(x?)(y?)\1\2', 'xy').groups())
"#,
    );
}

#[test]
fn template_errors() {
    common::interpreter_gate();
    check(
        "template-errors",
        r#"
import re
def show(fn, *a):
    try:
        return fn(*a)
    except Exception as e:
        return f'{type(e).__name__}: {e}'
print(show(lambda: re.sub('(a)', r'\2', 'a')))
print(show(lambda: re.sub('(a)', r'\g<zz>', 'a')))
print(show(lambda: re.sub('(a)', r'\q', 'a')))
"#,
    );
}

#[test]
fn dot_newline_edge() {
    common::interpreter_gate();
    check(
        "dot-newline",
        r#"
import re
print(re.match(r'.', '\n'))
print(re.match(r'(?s).', '\n').group())
print(re.findall(r'.', 'a\nb'))
print(re.findall(r'(?s).*', 'a\nb'))
print(re.match(r'.*b', 'a\nb', re.S).group())
"#,
    );
}
