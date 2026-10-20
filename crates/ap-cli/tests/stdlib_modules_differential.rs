//! stdlib_modules_differential — leaf 02.08-lite (T-11 companion).
//!
//! Per-module probes comparing our output byte-for-byte against a live
//! CPython 3.14 resolved through the shared reference-discovery seam
//! (`tests/common/mod.rs`: `AP_CONFORM_REFERENCE_PYTHON`, then PATH
//! `python3.14`; no bare `python3`, no hardcoded Homebrew path). Each test
//! skips (not fails) when python3.14 is unavailable — unless
//! `AP_REQUIRE_REFERENCE=1`, which turns a missing reference into a hard
//! failure (CI). Every test runs BOTH interpreters on the same source,
//! comparing stdout bytes + exit code.
//!
//! Cites: S-05-4 (v1-subset rows), S-05-5 (observable-behavior gate), the
//! spec 05 corpus anchors #4 (math), #5 (json), #7 (itertools), #8
//! (functools/collections), plus the 02.08-lite module list.

use std::process::Command;

mod common;

/// Run `src` under both interpreters; assert identical stdout + exit 0.
fn assert_differential(src: &str) {
    let py = match common::reference_or_skip() {
        Some(p) => p,
        None => return, // SKIP printed by the seam, or panic under
                        // AP_REQUIRE_REFERENCE=1.
    };
    let expected = Command::new(py)
        .arg("-S")
        .arg("-c")
        .arg(src)
        .output()
        .expect("spawn python3.14");
    // This test lives in the ap-cli package (which owns the ap-run-eval
    // bin), so CARGO_BIN_EXE_ap-run-eval resolves to a freshly built binary
    // in the SAME profile as the test. (It previously pointed at a hardcoded
    // target/release/ path, which made `cargo test` panic NotFound on trees
    // without a manual release build and silently diff against a STALE
    // release binary when one existed.)
    let bin = env!("CARGO_BIN_EXE_ap-run-eval");
    // Unique per test (tests run in parallel threads of one process).
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CASE_SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = CASE_SEQ.fetch_add(1, Ordering::SeqCst);
    let tmp = std::env::temp_dir().join(format!(
        "ap_stdlib_diff_case_{}_{}.py",
        std::process::id(),
        n
    ));
    std::fs::write(&tmp, src).expect("write temp case");
    let actual = Command::new(bin)
        .arg(&tmp)
        .output()
        .expect("spawn ap-run-eval");

    let expected_out = String::from_utf8_lossy(&expected.stdout).to_string();
    let actual_out = String::from_utf8_lossy(&actual.stdout).to_string();
    // Some probes legitimately exit non-zero (sys.exit cases): the gate is
    // that BOTH interpreters agree on the code.
    assert_eq!(
        expected.status.code(),
        actual.status.code(),
        "exit code mismatch for:\n{src}\nref-stderr: {}\nimpl-stderr: {}",
        String::from_utf8_lossy(&expected.stderr),
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        expected_out, actual_out,
        "stdout mismatch for:\n{src}\n-- expected --\n{expected_out}-- actual --\n{actual_out}"
    );
}

// ---------------------------------------------------------------------------
// math (spec 05 anchor #4 + error-text pins)
// ---------------------------------------------------------------------------

#[test]
fn math_basics_differential() {
    assert_differential(
        r#"
import math
print(math.isqrt(10**20), math.gcd(12, 18), math.pi, math.floor(2.7))
print(repr(math.tau), repr(math.e), repr(math.inf), repr(math.nan), repr(-math.inf))
print(repr(math.sqrt(2)), repr(math.pow(2, 0.5)), repr(math.exp(1)),
      repr(math.log(math.e)), repr(math.log2(8)), repr(math.log10(1000)),
      repr(math.log1p(0)), repr(math.expm1(0)), repr(math.cbrt(27)),
      repr(math.exp2(3)))
print(math.comb(5, 2), math.perm(5, 2), math.factorial(5), math.gcd(0, 7),
      math.lcm(4, 6), math.prod([2, 3]), math.prod(range(1, 6)))
print(math.dist([1, 2], [4, 6]), math.dist([0], [0]), math.hypot(3, 4))
print(math.fma(2, 3, 4), math.sumprod([1, 2], [3, 4]))
print(repr(math.nextafter(1.0, 2.0)), repr(math.ulp(1.0)))
print(repr(math.remainder(10, 3)), repr(math.fmod(7, 2)), repr(math.fabs(-2)),
      repr(math.ldexp(3, 2)), math.frexp(8.0), math.modf(-3.25))
print(repr(math.fsum([0.1] * 10)), repr(math.gamma(5)), repr(math.erf(1)),
      repr(math.erfc(1)), repr(math.lgamma(5)))
print(math.isclose(0.1 + 0.2, 0.3), math.isfinite(1.0), math.isinf(math.inf),
      math.isnan(math.nan))
print(math.copysign(1, -0.0), repr(math.ceil(-1.5)), math.floor(-1.5),
      math.trunc(-1.5))
print(math.isqrt(2**100), math.comb(100, 50) == 100891344545564193334812497256)
"#,
    );
}

#[test]
fn math_errors_differential() {
    assert_differential(
        r#"
import math
def exc(fn, *a):
    try:
        v = fn(*a)
        return f"{fn.__name__}-value {v!r}"
    except Exception as e:
        return f"{fn.__name__} {type(e).__name__}: {e}"
print(exc(math.sqrt, -1.0))
print(exc(math.exp, 1000.0))
print(exc(math.log, 0))
print(exc(math.isqrt, -4))
print(exc(math.factorial, -1))
print(exc(math.comb, -1, 2))
print(exc(math.perm, 5, -1))
print(exc(math.log, -1))
print(exc(math.acos, 2))
print(math.prod(iter([])), math.prod([0, 1, 2]), math.prod([], start=5))
print(math.isqrt(2**100) == 2**50, math.gcd(2**64, 2**32))
"#,
    );
}

// ---------------------------------------------------------------------------
// itertools (spec 05 anchor #7)
// ---------------------------------------------------------------------------

#[test]
fn itertools_differential() {
    assert_differential(
        r#"
import itertools, operator
print(list(itertools.islice(itertools.count(2), 3)),
      list(itertools.chain([1], [2])), list(itertools.batched("abc", 2)))
print(list(itertools.islice(itertools.count(1, 2), 4)))
print(list(itertools.islice(itertools.cycle("ab"), 5)))
print(list(itertools.repeat(7, 3)))
print(list(itertools.chain.from_iterable([[1], [], [2, 3]])))
print(list(itertools.accumulate([1, 2, 3, 4])))
print(list(itertools.accumulate([1, 2, 3, 4], operator.mul)))
print(list(itertools.compress("abcde", [1, 0, 1, 0, 1])))
print(list(itertools.dropwhile(lambda x: x < 3, [1, 2, 3, 1])))
print(list(itertools.takewhile(lambda x: x < 3, [1, 2, 3, 1])))
print(list(itertools.filterfalse(lambda x: x % 2, range(6))))
print(list(itertools.starmap(pow, [(2, 3), (3, 2)])))
print(list(itertools.pairwise("abcd")))
print(list(itertools.islice(itertools.tee(itertools.count(), 2)[1], 3)))
print(list(itertools.product("ab", [1, 2])))
print(list(itertools.product("ab", repeat=2)))
print(list(itertools.permutations([1, 2, 3], 2)))
print(list(itertools.combinations([1, 2, 3, 4], 2)))
print(list(itertools.combinations_with_replacement("ab", 2)))
print(list(itertools.zip_longest([1, 2], ["a"], fillvalue=0)))
print(list(itertools.islice(itertools.chain.from_iterable(
      itertools.repeat([1], 2)), 4)))
"#,
    );
}

// ---------------------------------------------------------------------------
// operator
// ---------------------------------------------------------------------------

#[test]
fn operator_differential() {
    assert_differential(
        r#"
import operator
print(operator.add(1, 2), operator.sub(5, 1), operator.mul(3, 4),
      operator.truediv(7, 2), operator.floordiv(7, 2), operator.mod(7, 2),
      operator.pow(2, 10))
print(operator.eq(1, 1), operator.ne(1, 2), operator.lt(1, 2),
      operator.le(2, 2), operator.gt(3, 2), operator.ge(3, 3))
print(operator.and_(0b1101, 0b0111), operator.or_(1, 2), operator.xor(5, 3),
      operator.lshift(1, 4), operator.rshift(16, 2))
print(operator.neg(-3), operator.pos(3), operator.abs(3),
      operator.invert(5))
print(operator.contains([1, 2, 3], 2), operator.countOf([1, 1, 2], 1))
print(operator.truth(0), operator.truth([0]), operator.not_(0),
      operator.is_(None, None), operator.is_not(1, 2))
print(operator.length_hint(range(10**6)), operator.index(3))
ig = operator.itemgetter(1, 0)
print(ig(["a", "b"]))
d = {"k": 1}
operator.setitem(d, "z", 2)
print(operator.getitem(d, "k"), d["z"])
print(operator.caller_check if False else "ok")
"#,
    );
}

// ---------------------------------------------------------------------------
// heapq / bisect (spec 05 §5.3.2 rows)
// ---------------------------------------------------------------------------

#[test]
fn heapq_bisect_differential() {
    assert_differential(
        r#"
import heapq, bisect
h = []
for v in [5, 3, 8, 1]:
    heapq.heappush(h, v)
print(h[0], [heapq.heappop(h) for _ in range(4)])
hh = [9, 4, 7]
heapq.heapify(hh)
print(hh, heapq.heappushpop(hh, 2), heapq.heapreplace(hh, 0))
print(heapq.nlargest(3, [1, 5, 3, 9, 2]), heapq.nsmallest(2, [1, 5, 3, 9, 2]))
print(list(heapq.merge([1, 4], [2, 3], [0])))
a = [1, 3, 5, 7]
print(bisect.bisect_left(a, 2), bisect.bisect(a, 5), bisect.bisect_right(a, 5))
bisect.insort(a, 4)
print(a)
bisect.insort_left(a, 4)
print(a)
print(bisect.bisect(a, 4, lo=1))
"#,
    );
}

// ---------------------------------------------------------------------------
// functools reduce/partial (spec 05 anchor #8 subset that needs no classes)
// ---------------------------------------------------------------------------

#[test]
fn functools_differential() {
    assert_differential(
        r#"
import functools
print(functools.reduce(lambda a, b: a + b, [1, 2, 3]),
      functools.reduce(lambda a, b: a + b, [], 10),
      functools.reduce(max, [3, 9, 1]))
def double(x):
    return 2 * x
p = functools.partial(double, 3)
print(p(), p.args)
addk = functools.partial(lambda a, b=0: a + b, 1)
print(addk())
"#,
    );
}

// ---------------------------------------------------------------------------
// os / os.path / posixpath / stat / errno / keyword (pure surface)
// ---------------------------------------------------------------------------

#[test]
fn os_path_differential() {
    assert_differential(
        r#"
import os
print(repr(os.sep), repr(os.pathsep), repr(os.curdir), repr(os.pardir),
      repr(os.extsep), repr(os.altsep), os.name)
print(os.error is OSError, os.devnull, os.SEEK_SET, os.SEEK_CUR, os.SEEK_END)
import posixpath
names = ["join", "split", "dirname", "basename", "exists", "isdir",
         "normpath", "abspath", "isabs", "expanduser", "sep", "curdir",
         "pardir", "altsep", "extsep", "pathsep"]
print(all(hasattr(posixpath, n) for n in names))
print(posixpath.join("a", "b"), posixpath.split("/x/y.zip"),
      posixpath.basename("/x/y.zip"), posixpath.dirname("/x/y.zip"))
print(posixpath.normpath("/x//y/../z"), posixpath.isabs("y"),
      posixpath.altsep is None, posixpath.sep, repr(posixpath.pathsep))
print(posixpath.exists("/no/such/path/t11"),
      posixpath.isdir("/no/such/path/t11"))
print(posixpath.splitext("/a/b.tar.gz"))
import os.path
print(posixpath.join is os.path.join, os.path.__name__ == "posixpath")
"#,
    );
}

#[test]
fn stat_errno_keyword_differential() {
    assert_differential(
        r#"
import errno, stat, keyword
print(stat.filemode(stat.S_IFDIR | 0o755))
print(stat.filemode(stat.S_IFREG | 0o644))
print(stat.filemode(stat.S_IFLNK | 0o777))
print(stat.S_ISDIR(stat.S_IFDIR), stat.S_ISREG(stat.S_IFREG),
      not stat.S_ISDIR(stat.S_IFREG))
print(errno.errorcode[errno.ENOENT])
print(errno.errorcode[errno.EEXIST], errno.errorcode[errno.EACCES])
print(isinstance(errno.errorcode, dict),
      all(str(n).startswith("E") for n in errno.errorcode.values()))
mode = stat.S_IFREG | 0o644
print(stat.filemode(mode)[-9:], (mode & stat.S_IRUSR) != 0,
      (mode & stat.S_IWOTH) == 0)
print(keyword.iskeyword("lambda"), keyword.iskeyword("_"),
      keyword.issoftkeyword("_"), keyword.issoftkeyword("case"),
      len(keyword.kwlist))
print(sorted(keyword.kwlist)[:4])
"#,
    );
}

// ---------------------------------------------------------------------------
// string / textwrap
// ---------------------------------------------------------------------------

#[test]
fn string_textwrap_differential() {
    assert_differential(
        r#"
import string, textwrap
print(string.digits, string.hexdigits, string.octdigits,
      string.ascii_uppercase[:3], len(string.ascii_letters),
      len(string.punctuation), repr(string.whitespace))
print(string.capwords("  a b  c "))
print(len(string.printable), repr(string.ascii_lowercase[:6]))
print(textwrap.wrap("the quick brown fox jumps", width=12))
print(textwrap.fill("the quick brown fox jumps", width=12).replace("\n", "|"))
print(textwrap.indent("a\nb", "  ").replace("\n", "|"), "END")
print(repr(textwrap.dedent("    a\n      b\n")))
print(textwrap.dedent("\nshared\nindent\n").strip().split())
"#,
    );
}

// ---------------------------------------------------------------------------
// json round-trip + errors (spec 05 anchor #5; error texts C-25-pinned)
// ---------------------------------------------------------------------------

#[test]
fn json_dumps_loads_differential() {
    assert_differential(
        r#"
import json
print(json.dumps({"b": 1, "a": [1, 2.5, True, None]}, sort_keys=True))
print(json.dumps({"b": 1, "a": 2}))
print(json.loads('{"x": [1, 2.0]}'))
print(json.loads('[1, 2.5, "x"]'))
d = json.loads('{"k1": 1, "k2": 2, "k0": 0}')
print(list(d.keys()))
print(json.dumps([1, 2], separators=(",", ":")))
print(json.dumps("café"), json.dumps("café", ensure_ascii=False))
print(json.dumps([float("inf"), float("-inf"), float("nan")]))
print(json.loads('[1e2]', parse_float=lambda x: 'PF') == ['PF'])
rt = {"s": "x", "i": 1, "f": [1, {"g": None}], "b": True}
print(json.loads(json.dumps(rt)) == rt)
print(json.dumps({"t": (1, 2)}), json.dumps({2.0: "float-key"}))
print(json.dumps({"esc": "a\"b\\c\td"}))
print(sorted(json.loads('{"z":1,"a":2,"m":3}',
      object_pairs_hook=lambda p: p)))
print(json.detect_encoding(b'{"a":1}'))
"#,
    );
}

#[test]
fn json_errors_differential() {
    assert_differential(
        r#"
import json
def show(s):
    try:
        json.loads(s)
        return "ok"
    except json.JSONDecodeError as e:
        return (f"{type(e).__name__}: {e} | msg={e.msg} L{e.lineno} "
                f"C{e.colno} P{e.pos} doc={e.doc!r}")
print(show("{"))
print(show('{"a":1,}'))
print(show("[1,"))
print(show("tru"))
print(show("nope"))
try:
    json.dumps({"a": {1, 2}})
except TypeError as e:
    print("TypeError:", e)
try:
    json.dumps(float("inf"), allow_nan=False)
except ValueError as e:
    print(type(e).__name__ + ":", e)
try:
    json.loads("{'a': 1}")
except json.JSONDecodeError as e:
    print("single-quote:", e.msg, e.colno)
print(issubclass(json.JSONDecodeError, ValueError))
print(json.dumps({"emoji": "\U0001F600"}))
print(json.dumps({"emoji": "\U0001F600"}, ensure_ascii=False))
"#,
    );
}

// ---------------------------------------------------------------------------
// atexit LIFO + unregister (spec 05 atexit anchor)
// ---------------------------------------------------------------------------

#[test]
fn atexit_differential() {
    assert_differential(
        r#"
import atexit, sys
print("main-start")
def a():
    print("A-last-registered-runs-first")
def b():
    print("B-first-registered-runs-second")
atexit.register(b)
atexit.register(a)
def un():
    print("UN-should-not-run")
atexit.register(un)
atexit.unregister(un)
print("unregistered", un.__name__)
atexit.register(print, "bye")
sys.exit(3)
"#,
    );
}

// ---------------------------------------------------------------------------
// random: MT19937 seed determinism (documented divergence vs S-05-19 OUT
// marking — implemented EXACTLY, so seeded sequences byte-match CPython)
// ---------------------------------------------------------------------------

#[test]
fn random_mt19937_differential() {
    assert_differential(
        r#"
import random
random.seed(0)
print([round(random.random(), 12) for _ in range(5)])
random.seed(42)
print(random.randint(1, 10), random.randint(1, 10))
random.seed(12345)
print([random.randrange(100) for _ in range(6)])
random.seed(7)
print(random.choice("abcdef"), random.uniform(1.0, 2.0))
"#,
    );
}

// ---------------------------------------------------------------------------
// hashlib digests (documented divergence vs S-05-19 OUT marking)
// ---------------------------------------------------------------------------

#[test]
fn hashlib_differential() {
    assert_differential(
        r#"
import hashlib
for name in ("md5", "sha1", "sha256", "sha512"):
    h = getattr(hashlib, name)(b"")
    print(name, h.hexdigest())
h = hashlib.sha256(b"antipythany")
print(h.hexdigest(), h.digest_size, h.block_size)
"#,
    );
}

// ---------------------------------------------------------------------------
// base64 / binascii hex (documented divergence vs S-05-20 OUT marking)
// ---------------------------------------------------------------------------

#[test]
fn base64_binascii_differential() {
    assert_differential(
        r#"
import base64, binascii
print(base64.b64encode(b"hello world"))
print(base64.b64decode(b"aGVsbG8gd29ybGQ="))
print(binascii.hexlify(b"\x00\x01\xfe\xff"))
print(binascii.unhexlify("0001feff"))
print(binascii.crc32(b"hello"))
"#,
    );
}

// ---------------------------------------------------------------------------
// time: UTC calendar surface (gmtime(0)/strftime/strptime — value-bearing
// wall-clock calls stay corpus-prohibited per spec 05 time row)
// ---------------------------------------------------------------------------

#[test]
fn time_utc_differential() {
    assert_differential(
        r#"
import time
gt = time.gmtime(0)
print(gt.tm_year, gt.tm_mon, gt.tm_mday, gt.tm_hour, gt.tm_min, gt.tm_sec,
      gt.tm_wday, gt.tm_yday, gt.tm_isdst)
print(tuple(gt))
print(time.strftime("%Y-%m-%d %H:%M:%S", gt))
print(time.strptime("2026-10-16", "%Y-%m-%d").tm_year)
st = time.struct_time((2020, 1, 2, 3, 4, 5, 3, 2, 0))
print(time.strftime("%Y-%m-%d %H:%M:%S", st))
print(time.gmtime(86400 * 365).tm_year)
"#,
    );
}

// ---------------------------------------------------------------------------
// linecache on a real temp file (getline round-trip)
// ---------------------------------------------------------------------------

#[test]
fn linecache_differential() {
    assert_differential(
        r#"
import linecache, tempfile, os
fd, path = tempfile.mkstemp(suffix=".t11")
f = open(path, "w")
f.write("alpha\nbeta\ngamma\n")
f.close()
print(repr(linecache.getline(path, 2)))
print(len(linecache.getlines(path)))
print(repr(linecache.getline("/definitely_absent_t11.py", 1)))
print(linecache.getlines("/definitely_absent_t11.py"))
linecache.clearcache()
print(len(linecache.cache) >= 0)
os.unlink(path)
"#,
    );
}

// ---------------------------------------------------------------------------
// collections.deque native type
// ---------------------------------------------------------------------------

#[test]
fn deque_differential() {
    assert_differential(
        r#"
import collections
d = collections.deque([1, 2, 3], maxlen=4)
print(d)
d.append(4)
d.append(5)
print(d, len(d))
d.appendleft(0)
print(d)
d.rotate(1)
print(d, d.popleft(), d.pop(), list(d))
print(collections.deque([1, 2]).index(2))
e = collections.deque()
e.extend([1, 2, 3])
e.clear()
print(list(e))
"#,
    );
}

// ---------------------------------------------------------------------------
// platform presence (values documented divergence — shapes only)
// ---------------------------------------------------------------------------

#[test]
fn platform_shapes_differential() {
    assert_differential(
        r#"
import platform
names = ["system", "machine", "node", "python_version",
         "python_implementation", "processor", "uname"]
print(all(callable(getattr(platform, n)) for n in names))
u = platform.uname()
print(all(hasattr(u, a) for a in
      ("system", "node", "release", "version", "machine", "processor")))
print(type(u.system).__name__ == "str")
print(sorted(u._fields) == sorted(["system", "node", "release", "version",
                                   "machine", "processor"]))
"#,
    );
}
