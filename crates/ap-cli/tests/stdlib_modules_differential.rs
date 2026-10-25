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

// ---------------------------------------------------------------------------
// A-1 (RustPython adaptation wave 1): errno table, binascii qp/uu/hqx,
// bytes.hex sep forms + fromhex, stat S_IMODE/S_IFMT, os.pipe/kill/utime.
// ---------------------------------------------------------------------------

#[test]
fn errno_full_table_differential() {
    assert_differential(
        r#"
import errno
for n in (1, 2, 13, 35, 107):
    print(n, errno.errorcode[n], errno.errorcode[n] != "")
print(errno.EWOULDBLOCK == errno.EAGAIN)
print(errno.ENOTCAPABLE > 100, errno.EDEADLK == 11)
print(len(errno.errorcode) >= 100)
"#,
    );
}

#[test]
fn binascii_qp_uu_hqx_differential() {
    assert_differential(
        r#"
import binascii
data = b"hello =20 world
"
enc = binascii.b2a_qp(data)
print(enc)
print(binascii.a2b_qp(enc))
print(binascii.a2b_qp(b"a=41b=7Ec=3Dd"))
print(binascii.b2a_qp(b"foo bar  ", istext=True))
print(binascii.crc_hqx(b"123456789", 0))
u = binascii.b2a_uu(b"Hell")
print(u)
print(binascii.a2b_uu(u))
print(binascii.Error.__name__, binascii.Incomplete.__name__)
try:
    binascii.unhexlify("zz")
except binascii.Error as e:
    print("caught", type(e).__name__, e)
"#,
    );
}

#[test]
fn bytes_hex_sep_fromhex_differential() {
    assert_differential(
        r#"
print(b"hello".hex())
print(b"hello".hex(":"))
print(b"hello".hex(":", 2))
print(b"hello".hex(":", -2))
print(b"hello".hex(":", 10))
print(bytes.fromhex("68 65"))
try:
    bytes.fromhex("6")
except ValueError as e:
    print("VE:", e)
try:
    bytes.fromhex("6z")
except ValueError as e:
    print("VE:", e)
import binascii
print(binascii.hexlify(b"abc"))
"#,
    );
}

#[test]
fn stat_imode_ifmt_differential() {
    assert_differential(
        r#"
import stat
print(stat.S_IMODE(0o100644))
print(oct(stat.S_IFMT(0o100644)))
print(stat.S_ISREG(0o100644), stat.S_ISDIR(0o040755))
"#,
    );
}

#[test]
fn os_pipe_kill_utime_differential() {
    assert_differential(
        r#"
import os
r_, w_ = os.pipe()
os.write(w_, b"ping")
print("pipe:", os.read(r_, 4))
os.close(r_)
os.close(w_)
try:
    os.kill(999999999, 15)
except OSError as e:
    print("kill-err:", type(e).__name__)
p = "/tmp/a1_gate_utime"
open(p, "w").write("x")
os.utime(p, ns=(1000000000, 2000000000))
st = os.stat(p)
print("utime:", st.st_atime, st.st_mtime)
os.utime(p, times=(5.5, 6.25))
st = os.stat(p)
print("utime times:", st.st_atime, st.st_mtime)
os.utime(p)
print("utime now ok")
os.unlink(p)
"#,
    );
}

#[test]
fn bytearray_fromhex_error_position_differential() {
    assert_differential(
        r#"
try:
    bytearray.fromhex("ab zz cd")
except ValueError as e:
    print("VE:", e)
try:
    bytearray.fromhex("abc")
except ValueError as e:
    print("VE:", e)
"#,
    );
}

// ---------------------------------------------------------------------------
// A-2 (RustPython adaptation wave 2): hashlib sha3/shake/blake2, zlib
// one-shot, os dup/dup2/fstat + stat dir_fd, os.uname structseq, realpath
// symlink resolution + '//' preservation, select darwin error taxonomy,
// utime ns-precision regression, utime tuple-length validation,
// kill EPERM taxonomy.
// ---------------------------------------------------------------------------

#[test]
fn hashlib_sha3_blake2_shake_differential() {
    assert_differential(
        r#"
import hashlib
print(hashlib.sha3_224(b"").hexdigest())
print(hashlib.sha3_256(b"abc").hexdigest())
print(hashlib.sha3_384(b"abc").hexdigest())
print(hashlib.sha3_512(b"abc").hexdigest())
print(hashlib.blake2b(b"abc").hexdigest())
print(hashlib.blake2s(b"abc").hexdigest())
print(hashlib.blake2b(b"abc", digest_size=32).hexdigest())
print(hashlib.blake2s(b"abc", digest_size=16).hexdigest())
def exc(fn, *a, **k):
    try:
        return "value " + repr(fn(*a, **k))
    except Exception as e:
        return type(e).__name__ + ": " + str(e)
print(exc(hashlib.blake2b, b"x", digest_size=65))
print(exc(hashlib.blake2s, b"x", digest_size=33))
# shake variable-length digests (XOF; length REQUIRED)
print(hashlib.shake_128(b"abc").hexdigest(16))
print(hashlib.shake_128(b"abc").hexdigest(32))
print(hashlib.shake_256(b"ab").hexdigest(20))
print(hashlib.shake_128(b"abc").digest(8).hex())
print(exc(lambda: hashlib.shake_128(b"x").hexdigest()))
# name/digest_size/block_size metadata
h = hashlib.sha3_256(b"abc")
print(h.name, h.digest_size, h.block_size)
b = hashlib.blake2b(b"abc")
print(b.name, b.digest_size, b.block_size)
s = hashlib.shake_128(b"abc")
print(s.name, s.digest_size, s.block_size)
# new() by name + unsupported name
print(hashlib.new("sha3_256", b"abc").hexdigest())
print(exc(hashlib.new, "nope"))
# algorithms sets
g = hashlib.algorithms_guaranteed
a = hashlib.algorithms_available
print(type(g).__name__, type(a).__name__)
print({"md5", "sha1", "sha256", "sha512", "sha224", "sha384", "sha3_224",
       "sha3_256", "sha3_384", "sha3_512", "shake_128", "shake_256",
       "blake2b", "blake2s"} <= g)
print(a >= g)
# hexdigest-callable regression: .digest() on a fixed-size hash object
h2 = hashlib.sha256(b"antipythany")
print(h2.hexdigest(), h2.digest().hex())
"#,
    );
}

#[test]
fn zlib_oneshot_differential() {
    assert_differential(
        r#"
import zlib
print(zlib.crc32(b"hello"), zlib.adler32(b"hello"))
print(zlib.crc32(b"hello", 5), zlib.adler32(b"hello", 5))
print(zlib.crc32(b""), zlib.adler32(b""))
data = b"hello world" * 100
c = zlib.compress(data)
print(zlib.decompress(c) == data)
print(zlib.decompress(zlib.compress(data, 1)) == data)
print(zlib.decompress(zlib.compress(data, 9)) == data)
print(zlib.decompress(zlib.compress(data, 0)) == data)
# raw deflate roundtrip through the one-shot surface (wbits=-15)
raw = zlib.compress(data, 6, -15)
print(zlib.decompress(raw, -15) == data)
# gzip container (wbits=31)
gz = zlib.compress(data, 6, 31)
print(zlib.decompress(gz, 31) == data)
def err(fn, *a):
    try:
        fn(*a)
        return "accepted"
    except zlib.error as e:
        return "zlib.error: " + str(e)
    except TypeError as e:
        return "TypeError: " + str(e)
print(err(zlib.decompress, b"not zlib data"))
print(err(zlib.compress, b"x", 12))
# Streaming objects are EXCLUDED (needs-redesign): only name presence is
# pinned (ours raises NotImplementedError where CPython returns an object).
print(hasattr(zlib, "compressobj"), hasattr(zlib, "decompressobj"))
print(zlib.MAX_WBITS, zlib.DEFLATED, zlib.Z_DEFAULT_COMPRESSION)
"#,
    );
}

#[test]
fn os_dup_dup2_fstat_differential() {
    assert_differential(
        r#"
import os
r_, w_ = os.pipe()
fd = os.dup(w_)
nfd = os.dup2(fd, 100)
os.close(w_)
os.write(nfd, b"shuffle")
print(os.read(r_, 8))
# dup2 to the SAME fd returns it unchanged (no close)
print(os.dup2(fd, fd) == fd)
# fstat on a real fd: same stat_result surface as the path forms
st = os.fstat(r_)
print(st.st_mode > 0, st.st_size >= 0, st.st_nlink >= 1)
print(st.st_mode == os.stat("/dev/stdin").st_mode if os.path.exists("/dev/stdin") else True)
# stat dir_fd via a real directory fd
dfd = os.open("/tmp", os.O_RDONLY)
f = open("/tmp/a2_dirfd_file", "w")
f.write("z")
f.close()
st2 = os.stat("a2_dirfd_file", dir_fd=dfd)
print(st2.st_size == 1)
os.close(dfd)
os.unlink("/tmp/a2_dirfd_file")
# follow_symlinks=False on stat == lstat
os.symlink("/etc/hosts", "/tmp/a2_follow_link")
st3 = os.stat("/tmp/a2_follow_link", follow_symlinks=False)
print(st3.st_mode & 0o170000 == 0o120000)
print(os.lstat("/tmp/a2_follow_link").st_mode == st3.st_mode)
os.unlink("/tmp/a2_follow_link")
def exc(fn, *a):
    try:
        fn(a[0] if a else None)
        return "accepted"
    except OSError as e:
        return "OSError " + str(e.errno)
os.close(r_)
os.close(fd)
os.close(nfd)
print(exc(os.dup, -5))
print(exc(os.fstat, 9999))
"#,
    );
}

#[test]
fn os_uname_structseq_differential() {
    assert_differential(
        r#"
import os
u = os.uname()
# structseq type name pins 'uname_result' only in CPython; ours renders the
# Opaque fallback ('object') — pin the observable value surface instead.
print("uname_result" in str(u))
print(u.sysname == "Darwin", u.machine == "arm64", u.nodename != "")
print(len(u) == 5)
print(u.n_fields, u.n_sequence_fields, u.n_unnamed_fields)
print(str(u).startswith("posix.uname_result(sysname="))
print(tuple(u)[0] == u.sysname)
# platform.uname forwards from os.uname on posix + appends processor
import platform
pu = platform.uname()
print(pu.system == "Darwin", pu.machine == "arm64")
print(len(pu) == 6, hasattr(pu, "processor"))
print(sorted(pu._fields) == sorted(["system", "node", "release",
                                    "version", "machine", "processor"]))
"#,
    );
}

#[test]
fn realpath_symlink_chain_differential() {
    assert_differential(
        r#"
import os, os.path, tempfile
d = tempfile.mkdtemp()
p = d + "/t"
open(p + "_target", "w").write("x")
os.symlink(p + "_target", p + "_l1")
os.symlink(p + "_l1", p + "_l2")
# symlink chain resolves to the real target
print(os.path.realpath(p + "_l2", strict=True).endswith("_target"))
print(os.path.realpath(p + "_l2").endswith("_target"))
# intermediate components through the link
print(os.path.realpath(p + "_l1" + "/miss/deep").endswith("_target/miss/deep"))
# '//' semantics: normpath PRESERVES the double-leading slash,
# realpath collapses it (CPython 3.14 probes)
print(os.path.normpath("//a/b"))
print(os.path.normpath("///a/b"))
print(os.path.realpath("//a/b"))
# strict=True error surface
def exc(fn, *a, **k):
    try:
        return "value"
    except OSError as e:
        return type(e).__name__ + " " + str(e.errno)
print(exc(os.path.realpath, "/definitely_missing_a2", strict=True))
# dangling symlink
os.symlink("/nonexistent_a2", p + "_dang")
print(os.path.realpath(p + "_dang") == "/nonexistent_a2")
print(exc(os.path.realpath, p + "_dang", strict=True))
# symlink loop
os.symlink(p + "_la", p + "_lb")
os.symlink(p + "_lb", p + "_la")
print(exc(os.path.realpath, p + "_la", strict=True))
# non-final component not a directory under strict
print(exc(os.path.realpath, p + "_target" + "/sub", strict=True))
os.unlink(p + "_dang")
os.unlink(p + "_la")
os.unlink(p + "_lb")
os.unlink(p + "_l1")
os.unlink(p + "_l2")
os.unlink(p + "_target")
"#,
    );
}

#[test]
fn select_unsupported_platform_errors_differential() {
    assert_differential(
        r#"
import select, os
# PROBED python3.14-darwin: epoll/devpoll are ABSENT (AttributeError on
# access), select.error IS OSError, poll exists and works.
print(hasattr(select, "epoll"), hasattr(select, "devpoll"))
print(select.error is OSError)
print(hasattr(select, "poll"), hasattr(select, "kqueue"))
# poll object: real poll(2) semantics; CPython's select.poll has NO close
pp = select.poll()
pr, pw = os.pipe()
pp.register(pr, select.POLLIN)
print(pp.poll(0))
print(hasattr(pp, "close"))
def exc(fn, *a):
    try:
        fn(*a)
        return "accepted"
    except Exception as e:
        return type(e).__name__ + " " + str(getattr(e, "errno", e))
print(exc(pp.modify, 99999, select.POLLIN))
print(exc(pp.unregister, 99999))
try:
    pp.register(-7, select.POLLIN)
except ValueError as e:
    print("VE:", e)
# poll(2) wakeup roundtrip through a second fd
pr2, pw2 = os.pipe()
pp.register(pr2, select.POLLIN)
os.write(pw2, b"x")
print(pp.poll(200) == [(pr2, select.POLLIN)])
pp.unregister(pr)
pp.unregister(pr2)
print(pp.poll(0))
os.close(pr)
os.close(pw)
os.close(pr2)
os.close(pw2)
"#,
    );
}

#[test]
fn utime_ns_precision_regression_differential() {
    assert_differential(
        r#"
import os
p = "/tmp/a2_ut_ns_prec"
open(p, "w").write("x")
os.utime(p, ns=(1600000000000000123, 1600000000000000123))
st = os.stat(p)
print(st.st_atime_ns, st.st_mtime_ns)
print(st.st_atime, st.st_mtime)
os.utime(p, times=(5.5, 6.25))
st = os.stat(p)
print(st.st_atime, st.st_mtime)
# tuple-length validation (wave-1 residual: 3-tuples rejected)
def exc(fn, *a, **k):
    try:
        fn(*a, **k)
        return "accepted"
    except Exception as e:
        return type(e).__name__ + ": " + str(e)
print(exc(os.utime, p, (1, 2, 3)))
print(exc(os.utime, p, ns=(1, 2, 3)))
print(exc(os.utime, p, ns=(1, 2), times=(1, 2)))
os.unlink(p)
"#,
    );
}

#[test]
fn kill_eperm_taxonomy_regression_differential() {
    assert_differential(
        r#"
import os
# EPERM maps to PermissionError like CPython (wave-1 residual).
try:
    os.kill(1, 0)
except PermissionError as e:
    print("PE", e.errno)
except OSError as e:
    print("OE", e.errno)
# dead pid keeps ProcessLookupError
try:
    os.kill(999999999, 15)
except ProcessLookupError:
    print("PLE")
except OSError as e:
    print("OE", e.errno)
"#,
    );
}

#[test]
fn bytearray_fromhex_type_regression_differential() {
    // KNOWN DIVERGENCE (reported, not fixed in A-2): the Value model has no
    // distinct bytearray type, so bytearray.fromhex returns a bytes-typed
    // object. That case cannot be differential-pinned until the object model
    // grows a bytearray type (finding filed in the A-2 report); this test
    // pins the bytes-side contract that DOES hold on both interpreters.
    assert_differential(
        r#"
b = bytes.fromhex("ab")
print(type(b).__name__, repr(b))
ba = bytearray(b"ab")
print(len(ba) == 2)
"#,
    );
}


#[test]
fn a3_except_exception_catches_native_module_errors_differential() {
    // A-3 item 2: zlib.Error / binascii.Error are real exception TypeObjects;
    // bare `except Exception` must walk the MRO and catch them (previously
    // the except matcher skipped them and the traceback escaped, rc=1).
    assert_differential(
        r#"
import binascii
import zlib


def catch_generic(fn, *a):
    try:
        fn(*a)
    except Exception as e:
        return "caught " + type(e).__name__ + ": " + str(e)
    return "uncaught"


print(catch_generic(zlib.decompress, b"abc"))
print(catch_generic(binascii.unhexlify, b"abc"))
# specific except still binds
try:
    zlib.decompress(b"abc")
except zlib.error as e:
    print("specific", type(e).__name__)


class MyWrap(Exception):
    pass


try:
    raise binascii.Error("x")
except Exception as e:
    print("raised-generic", isinstance(e, Exception))
"#,
    );
}

#[test]
fn a3_oserror_str_format_differential() {
    // A-3 item 3: str(OSError(errno, strerror)) is "[Errno N] msg" like
    // CPython, not the tuple repr; with a filename arg it appends ": 'path'".
    assert_differential(
        r#"
import os

try:
    os.kill(1, 0)
except PermissionError as e:
    print(type(e).__name__, str(e), e.errno)

try:
    os.open("/tmp/definitely-missing-antipythany-a3", os.O_RDONLY)
except FileNotFoundError as e:
    print(type(e).__name__, str(e), e.errno)

e = OSError(1, "Operation not permitted")
print(str(e))
"#,
    );
}

#[test]
fn a3_opaque_type_naming_differential() {
    // A-3 item 5: type(os.uname()) reports uname_result (not object).
    assert_differential(
        r#"
import os
print(type(os.uname()).__name__)
"#,
    );
}

#[test]
fn a3_stat_result_sequence_differential() {
    // A-3 item 5: stat_result is indexable and sized like a plain sequence
    // (st[0] == st.st_mode, len(st) >= 10).
    assert_differential(
        r#"
import os
st = os.stat("/tmp")
print(st[0] == st.st_mode)
print(len(st) >= 10)
"#,
    );
}

#[test]
fn a3_symlink_readlink_roundtrip_differential() {
    // A-3 item 1 verify: os.symlink real arm + os.readlink roundtrip.
    assert_differential(
        r#"
import os
import tempfile

d = tempfile.mkdtemp()
p = os.path.join(d, "link")
os.symlink("/etc/hosts", p)
print(os.path.islink(p))
print(os.readlink(p))
os.unlink(p)
print(os.path.exists(p))
"#,
    );
}

#[test]
fn a3_utime_three_tuple_typeerror_differential() {
    // A-3 item 4 verify: utime(p, (1, 2, 3)) raises the pinned TypeError.
    assert_differential(
        r#"
import os
import tempfile

d = tempfile.mkdtemp()
p = os.path.join(d, "f")
open(p, "w").close()
try:
    os.utime(p, (1, 2, 3))
    print("accepted")
except TypeError as e:
    print("TypeError:", e)
os.utime(p, (1, 2))
print("pair ok")
"#,
    );
}
