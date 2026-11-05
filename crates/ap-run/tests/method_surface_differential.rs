//! Leaf 02.08-full part 1 — method-surface differential probes vs live
//! CPython 3.14. Each probe runs the same snippet through the reference
//! interpreter and the tier-0 executor and requires byte-exact stdout.
//!
//! Families: str, bytes/bytearray, dict (+views), list, tuple,
//! set/frozenset, int/float. ~40 probes pinning the surfaces the leaf
//! implemented (rsplit ordering, universal-newline splitlines, format-spec
//! delegation, dict-view reprs/dynamism, set ops, int.from_bytes classmethod
//! forms, float.as_integer_ratio/hex/is_integer).

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
    // (CARGO_BIN_EXE_), serialized by the heavy-gate flock (see
    // tests/common/mod.rs).
    let (out, _err, _code) = common::run_aprun_eval(src);
    out
}

fn check(name: &str, src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    let src_file = std::env::temp_dir().join(format!("ms_differential_{name}.py"));
    std::fs::write(&src_file, src).expect("write probe");
    let reference = run_reference(py, src);
    let ours = run_aprun(src); // helper takes SOURCE, not the path
    assert_eq!(
        reference,
        ours,
        "probe {name}: stdout mismatch\n ref: {}\n ours: {}",
        String::from_utf8_lossy(&reference),
        String::from_utf8_lossy(&ours)
    );
}

// ----------------------------------------------------------------- str ---

#[test]
fn str_split_rsplit_family() {
    check(
        "str-split-rsplit",
        r#"
print("a-b-c".rsplit("-", 1))
print("a-b-c".rsplit("-"))
print("a-b-c".split("-", 1))
print("a b c".split(None, 1))
print("a,b,,c".split(","))
"#,
    );
}

#[test]
fn str_case_casefold_swapcase() {
    check(
        "str-case",
        r#"
print("Hello".casefold(), "aBc".swapcase(), "ß".casefold())
"#,
    );
}

#[test]
fn str_zfill_center_ljust_rjust() {
    check(
        "str-pad",
        r#"
print("5".zfill(3), "-5".zfill(4), "a".center(5), "a".center(6, "*"))
print("a".ljust(3), "a".rjust(3), "ab".ljust(1))
"#,
    );
}

#[test]
fn str_partition_rpartition() {
    check(
        "str-partition",
        r#"
print("a,b".partition(","), "a,b".rpartition(","))
print("abc".rpartition(","))
"#,
    );
}

#[test]
fn str_splitlines_universal() {
    check(
        "str-splitlines",
        r#"
print("a\r\nb\r\n".splitlines())
print("a\rb".splitlines())
print("a\x85b".splitlines())
print("".splitlines())
print("a\n".splitlines())
"#,
    );
}

#[test]
fn str_predicates() {
    check(
        "str-predicates",
        r#"
print("abc".isascii(), "a1".isidentifier(), "_x".isidentifier())
print("ab ".isprintable(), "12".isnumeric(), "12".isdecimal())
"#,
    );
}

#[test]
fn str_maketrans_translate() {
    check(
        "str-translate",
        r#"
t = str.maketrans("ab", "xy")
print(t, "abc".translate(t))
print("abc".translate(str.maketrans("a", "b", "c")))
"#,
    );
}

#[test]
fn str_expandtabs() {
    check(
        "str-expandtabs",
        "print(\"a\\tb\".expandtabs(), \"a\\tb\".expandtabs(4))",
    );
}

#[test]
fn str_startswith_endargs() {
    check(
        "str-startend",
        r#"
print("abc".startswith("b", 1), "abc".startswith(("x", "b"), 1))
print("abc".endswith("c", 0, 3))
"#,
    );
}

#[test]
fn str_format_full() {
    check(
        "str-format",
        r#"
print("{x}".format(x=5), "{}{}".format(1, 2), "{0}{0}".format("ab"))
print("{!r}".format("s"), "{:>5}|{:.2f}|{:#x}|{:,}".format(7, 3.14159, 255, 1234567))
"#,
    );
}

#[test]
fn str_rindex() {
    check(
        "str-rindex",
        r#"
print("abcabc".rindex("b"))
try:
    "abc".rindex("z")
except ValueError as e:
    print("VE:", e)
"#,
    );
}

// -------------------------------------------------------------- bytes ---

#[test]
fn bytes_strip_split() {
    check(
        "bytes-strip-split",
        r#"
print(b" ab ".strip(), b"a,b".split(b","), b"a,b".rsplit(b",", 1))
"#,
    );
}

#[test]
fn bytes_find_replace_join() {
    check(
        "bytes-find-replace-join",
        r#"
print(b"abc".find(b"b"), b"abc".rfind(b"c"), b"a-b".replace(b"-", b"+"))
print(b"a".join([b"x", b"y"]), b"ab".hex())
"#,
    );
}

#[test]
fn bytes_startswith_tuple() {
    check(
        "bytes-startswith",
        r#"
print(b"ab".startswith((b"x", b"a")), b"abc".endswith(b"bc"))
"#,
    );
}

#[test]
fn bytes_ctor_forms() {
    check(
        "bytes-ctor",
        r#"
print(bytes(3), bytes([1, 2]), bytes("ab", "utf-8"))
"#,
    );
}

// ---------------------------------------------------------------- dict ---

#[test]
fn dict_update_forms() {
    check(
        "dict-update",
        r#"
d = {}
d.update({1: 2})
d.update(x=3)
d.update([(4, 5)])
print(d)
"#,
    );
}

#[test]
fn dict_fromkeys() {
    check(
        "dict-fromkeys",
        r#"
print(dict.fromkeys([1, 2], 0), {1: 2}.fromkeys([3], 9))
"#,
    );
}

#[test]
fn dict_view_repr_iter() {
    check(
        "dict-views",
        r#"
print({1: 2}.items())
print(list({1: 2, 3: 4}.items()))
"#,
    );
}

#[test]
fn dict_view_dynamic() {
    check(
        "dict-view-dynamic",
        r#"
d = {1: 2}
v = d.items()
d[3] = 4
print(list(v))
"#,
    );
}

// ----------------------------------------------------------------- set ---

#[test]
fn set_ops_surface() {
    check(
        "set-ops",
        r#"
print({1}.union({2}, {3}), {1}.intersection({1, 2}))
print({1}.issubset({1, 2}), {1}.issuperset({1}), {1}.isdisjoint({2}))
print({1}.symmetric_difference({2}))
"#,
    );
}

#[test]
fn set_update_diff_update() {
    check(
        "set-update",
        r#"
s = {1}
s.update([2])
print(s)
s = {1, 2}
s.difference_update({1})
print(s)
"#,
    );
}

#[test]
fn set_remove_keyerror() {
    check(
        "set-remove",
        r#"
s = {1}
try:
    s.remove(2)
except KeyError as e:
    print("KE:", e)
"#,
    );
}

// --------------------------------------------------------------- tuple ---

#[test]
fn tuple_count_index() {
    check(
        "tuple-count-index",
        r#"
print((1, 2).count(1), (1, 2, 1).index(1, 1))
print((1, 2, 3).index(3))
"#,
    );
}

// ------------------------------------------------------- int / float ---

#[test]
fn int_from_bytes_classmethod() {
    check(
        "int-from-bytes",
        r#"
print(int.from_bytes(b"\x01\x00", "little"))
print(int.from_bytes(b"\xff", "big", signed=True))
print(int.from_bytes([1, 0], "little"))
"#,
    );
}

#[test]
fn int_bit_count() {
    check("int-bit-count", "print((255).bit_count(), (0).bit_count())");
}

#[test]
fn float_as_integer_ratio() {
    check(
        "float-ratio",
        r#"
print((3.5).as_integer_ratio(), (2.0).as_integer_ratio(), (0.1).as_integer_ratio())
"#,
    );
}

#[test]
fn float_hex_is_integer() {
    check(
        "float-hex",
        r#"
print((2.0).hex(), (-2.5).hex())
print((3.0).is_integer(), (2.5).is_integer())
"#,
    );
}

// ------------------------------------------ unbound type-level access ---

#[test]
fn unbound_builtin_type_methods() {
    check(
        "unbound-methods",
        r#"
print(list.copy([1, 2]), dict.get({1: 2}, 1), str.upper("ab"))
print(set.union({1}, {2}))
"#,
    );
}

// ---------------------------------------------------------- complex ---

#[test]
fn complex_repr_and_real() {
    check(
        "complex",
        r#"
print(repr(1j), repr(3 + 4j))
import operator
print(operator.attrgetter("real")(3 + 4j))
"#,
    );
}

// ------------------------------------------------- regression sentinels ---

#[test]
fn regress_enumerate_start_kwarg() {
    check(
        "enumerate-start",
        r#"
print(list(enumerate("ab", start=10)))
"#,
    );
}

#[test]
fn regress_syntaxerror_msg() {
    check(
        "syntaxerror-msg",
        r#"
try:
    eval("1+")
except SyntaxError as e:
    print(type(e).__name__, e.msg)
"#,
    );
}

#[test]
fn regress_ioerror_alias() {
    check(
        "ioerror-alias",
        r#"
print(IOError is OSError, EnvironmentError is OSError)
"#,
    );
}

#[test]
fn regress_version_info_slice() {
    check(
        "version-info",
        r#"
import sys
print(sys.version_info[:2] == (3, 14), sys.version_info.releaselevel)
print(sys.getfilesystemencoding(), sys.getfilesystemencodeerrors())
"#,
    );
}

#[test]
fn regress_stringio_text_mode() {
    check(
        "stringio-text",
        r#"
import io
s = io.StringIO("line1\nline2\n")
print(s.readline().strip(), s.read().strip())
s.seek(0)
print([ln.strip() for ln in s])
"#,
    );
}
