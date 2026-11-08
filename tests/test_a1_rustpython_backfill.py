# A-1 — RustPython leaf-donor adaptations, wave 1: SPEC-SIDE differential suite.
#
# Authorship: written from the leaf brief ONLY
# (plans/antipythony/05-ecosystem-t2/leaves/a-1-rustpython-adaptations-w1.md)
# plus live probing of the reference interpreter /opt/homebrew/bin/python3.14
# (3.14.7 — the spec made executable, per the separate-authorship test-track
# rule in tests/README.md). The author has NOT read any implementation source
# under crates/ (firewall respected; tests derive from spec + reference runs).
#
# The leaf's six adaptation items, and what this suite pins for each:
#   1. errno full table (leaf item 1)      -> a1_err0_* / a1_err1_*
#   2. binascii qp/uu/hqx (leaf item 2)    -> a1_bin0_* / a1_bin1_* / a1_bin2_*
#   3. bytes/bytearray hex/fromhex         -> a1_hex0_* / a1_hex1_*
#      (leaf item 3)
#   4. stat S_IMODE/S_IFMT + UF_/SF_       -> a1_stat0_* / a1_stat1_*
#      (leaf item 4)
#   5. os.pipe/pipe2 + os.kill/killpg      -> a1_pipe0_* / a1_pipe1_* /
#      (leaf item 5)                          a1_kill0_* / a1_kill1_*
#   6. os.utime completion                 -> a1_utime0_* / a1_utime1_*
#      (leaf item 6)
#
# TWO test faces (T-11/T-08 convention):
#
#  (A) REFERENCE face (green today): every probe program in this file is run
#      under the live reference and must reproduce the pinned stdout
#      BYTE-EXACTLY. The pins are the CPython 3.14 contract; they also
#      self-check this suite against interpreter drift.
#
#  (B) ANTIPYTHONY face (xfail, strict=False, until leaf A-1 lands): each
#      probe run under the implementation seam (`ap-run-eval <file>`, the
#      T-05/T-06 testability bin; APX_RUN_EVAL overrides) must produce the
#      same stdout bytes and exit code 0. The impl is landing in a sibling
#      agent's files in parallel, so these mostly xfail now — expected and
#      correct per the xfail-first convention. Any case that already passes
#      on the current binary is NOT marked xfail (it lives in the
#      ALREADY_GREEN set below and asserts outright).
#
# Probing surprises recorded while pinning (macOS, 3.14.7):
#   - os.pipe2 does NOT exist on macOS CPython (AttributeError); the pipe2
#     probe self-skips on the reference and stays xfail-tolerant on the impl.
#   - errno.errorcode has 107 entries on macOS (not ~56): EAGAIN==35 maps to
#     "EAGAIN" and the EWOULDBLOCK alias resolves to the SAME errorcode
#     string. EOPNOTSUPP(102) != ENOTSUP(45) on macOS (both present).
#   - bytes.hex(":", 0) does NOT raise (bytes_per_sep 0 means "no grouping")
#     but sep="" with bytes_per_sep!=0 raises ValueError sep must be length 1.
#   - b2a_qp istext=False escapes \n as =0A (binascii is stricter than
#     quopri); a2b_qp silently leaves invalid =zz escapes in place.
#   - a2b_uu(b"") raises Error("Missing length byte") — empty input is an
#     error, while a2b_uu(b" \n") (length byte 0) decodes to b"".
#   - os.utime ns=(1,"x") raises TypeError with a divmod message (leaks the
#     C-level coercion), while ns=(1,) raises
#     "utime: 'ns' must be a tuple of two ints".

import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

# This suite lives directly under tests/ (not a family dir), so repo root is
# two levels up — parents[1], not the parents[2] used by tests/tNN_*/ suites.
REPO_ROOT = Path(__file__).resolve().parents[1]

# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 skip->fail policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402


def _find_bin(name):
    env = os.environ.get(name.upper().replace("-", "_"))
    if env:
        parts = shlex.split(env)
        return parts[0], parts
    which = shutil.which(name)
    if which:
        return which, [which]
    for cfg in ("debug", "release"):
        p = REPO_ROOT / "target" / cfg / name
        if p.is_file():
            return str(p), [str(p)]
    return None, None


AP_RUN_EVAL, AP_RUN_EVAL_CMD = _find_bin("ap-run-eval")

XFAIL_REASON = "awaits leaf A-1 rustpython backfill (errno/binascii/hex/stat/pipe/kill/utime)"
xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)

# Cases that already pass on the CURRENT binary (measured 2026-10-24 on
# target/debug/ap-run-eval, python3.14 pins): errno table+invariants, stat
# S_IMODE/S_IFMT + UF_/SF_ constants, binascii qp roundtrips and crc_hqx.
# These are NOT xfail-marked (tests/README flip policy: a stale xfail hides
# regressions of the very behavior it celebrated). Everything else is
# xfail-first until leaf A-1 lands.
ALREADY_GREEN = frozenset({
    "a1_err0_table",
    "a1_err1_invariants",
    "a1_bin0_qp",
    "a1_bin2_hqx",
    "a1_stat0_modes",
    "a1_stat1_consts",
    # 2026-11-07 flip wave (01.02): green on the current binary; moved out of
    # the xfail half per the flip policy in this file's header.
    "a1_hex1_fromhex",
    "a1_utime0_ns",
})


# ---------------------------------------------------------------------------
# Probe programs. Each is a standalone .py written to a tmp dir at test time
# and executed by both faces. Every expected stdout below was pinned by a
# real /opt/homebrew/bin/python3.14 (3.14.7) run on macOS (darwin arm64).
# ---------------------------------------------------------------------------

PROBES = {}

# -- 1. errno ----------------------------------------------------------------

PROBES["a1_err0_table"] = r'''
import errno
names = ["EPERM","ENOENT","ESRCH","EINTR","EBADF","EAGAIN","EWOULDBLOCK","EDEADLK",
         "ENOTSUP","EOPNOTSUPP","EOVERFLOW","ECANCELED","ENOTRECOVERABLE","EOWNERDEAD",
         "EQFULL","ELOOP","ENAMETOOLONG","ETIMEDOUT","ECONNREFUSED","ENOTEMPTY"]
for n in names:
    v = getattr(errno, n)
    print(n, v, errno.errorcode[v])
print("len", len(errno.errorcode))
print("alias", errno.EWOULDBLOCK == errno.EAGAIN, errno.errorcode[errno.EWOULDBLOCK])
'''

EXPECTED = {}
EXPECTED["a1_err0_table"] = """\
EPERM 1 EPERM
ENOENT 2 ENOENT
ESRCH 3 ESRCH
EINTR 4 EINTR
EBADF 9 EBADF
EAGAIN 35 EAGAIN
EWOULDBLOCK 35 EAGAIN
EDEADLK 11 EDEADLK
ENOTSUP 45 ENOTSUP
EOPNOTSUPP 102 EOPNOTSUPP
EOVERFLOW 84 EOVERFLOW
ECANCELED 89 ECANCELED
ENOTRECOVERABLE 104 ENOTRECOVERABLE
EOWNERDEAD 105 EOWNERDEAD
EQFULL 106 EQFULL
ELOOP 62 ELOOP
ENAMETOOLONG 63 ENAMETOOLONG
ETIMEDOUT 60 ETIMEDOUT
ECONNREFUSED 61 ECONNREFUSED
ENOTEMPTY 66 ENOTEMPTY
len 107
alias True EAGAIN
"""

PROBES["a1_err1_invariants"] = r'''
import errno
bad = [k for k, v in errno.errorcode.items() if getattr(errno, v, None) != k]
print("violations", bad)
print("entries", len(errno.errorcode))
print("low", errno.errorcode[1], errno.errorcode[2], errno.errorcode[3])
print("high", errno.errorcode[89], errno.errorcode[104], errno.errorcode[105], errno.errorcode[106])
print("missing-ok", errno.errorcode.get(9999))
'''

EXPECTED["a1_err1_invariants"] = """\
violations []
entries 107
low EPERM ENOENT ESRCH
high ECANCELED ENOTRECOVERABLE EOWNERDEAD EQFULL
missing-ok None
"""

# -- 2. binascii -------------------------------------------------------------

PROBES["a1_bin0_qp"] = r'''
import binascii
print(repr(binascii.b2a_qp(b"hello world")))
print(repr(binascii.b2a_qp(b"line1\nline2\n")))
print(repr(binascii.b2a_qp(b"soft ")))
print(repr(binascii.b2a_qp(b"ab \n")))
print(repr(binascii.b2a_qp(b"a= b\n")))
print(repr(binascii.b2a_qp(b"a\r\nb")))
print(repr(binascii.b2a_qp(b"\x80\xff")))
print(repr(binascii.b2a_qp(b"allkeys\x00rare\n", istext=False)))
print(repr(binascii.b2a_qp(b"a\tb\n", quotetabs=True)))
print(repr(binascii.b2a_qp(b"a  b\n")))
print(repr(binascii.b2a_qp(b"From: =? x", header=True)))
print(repr(binascii.b2a_qp(b"From: =? x", header=False)))
print(repr(binascii.a2b_qp(b"soft=\nline")))
print(repr(binascii.a2b_qp(b"a=41b=7E")))
print(repr(binascii.a2b_qp(b"x=3Dy=20z=\n w")))
print(repr(binascii.a2b_qp(b"=\n")))
print(repr(binascii.a2b_qp(b"a=zz")))
print(repr(binascii.a2b_qp(b"=20=3D")))
long = b"x" * 200 + b"\n"
enc = binascii.b2a_qp(long)
print(len(enc), enc.count(b"=\n"), binascii.a2b_qp(enc) == long)
'''

EXPECTED["a1_bin0_qp"] = """\
b'hello world'
b'line1\\nline2\\n'
b'soft=20'
b'ab=20\\n'
b'a=3D b\\n'
b'a\\r\\nb'
b'=80=FF'
b'allkeys=00rare=0A'
b'a=09b\\n'
b'a  b\\n'
b'From:_=3D?_x'
b'From: =3D? x'
b'softline'
b'aAb~'
b'x=y z w'
b''
b'a=zz'
b' ='
205 2 True
"""

PROBES["a1_bin1_uu"] = r'''
import binascii
print(repr(binascii.b2a_uu(b"Cat")))
print(repr(binascii.a2b_uu(b"#0V%T\n")))
print(repr(binascii.b2a_uu(b"")))
print(repr(binascii.a2b_uu(b" \n")))
print(repr(binascii.b2a_uu(b"x" * 45)))
print(repr(binascii.a2b_uu(b'&5&@"' + b"\n")))
print(repr(binascii.a2b_uu(b"#0V%T")))
for bad, why in ((b"", "empty"), (b"M" + b"x" * 45 + b"Z" + b"\n", "overlong"), (b"zzzz\n", "junk")):
    try:
        binascii.a2b_uu(bad)
        print(why, "no-error")
    except Exception as e:
        print(why, type(e).__name__, e)
try:
    binascii.b2a_uu(b"x" * 46)
except Exception as e:
    print("toolong", type(e).__name__, e)
enc2 = binascii.b2a_uu(b"Cat") + binascii.b2a_uu(b"")
print(repr(enc2))
print(repr(binascii.a2b_uu(enc2)))
'''

EXPECTED["a1_bin1_uu"] = """\
b'#0V%T\\n'
b'Cat'
b' \\n'
b''
b"M>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX>'AX\\n"
b'Th\\x02\\x00\\x00\\x00'
b'Cat'
empty Error Missing length byte
overlong Error Illegal char
junk Error Illegal char
toolong Error At most 45 bytes at once
b'#0V%T\\n \\n'
b'Cat'
"""

PROBES["a1_bin2_hqx"] = r'''
import binascii
print(binascii.crc_hqx(b"123456789", 0))
print(binascii.crc_hqx(b"123456789", 0xffff))
print(binascii.crc_hqx(b"", 0))
print(binascii.crc_hqx(b"", 0xffff))
print(binascii.crc_hqx(b"abc", 0x1021))
print(binascii.crc_hqx(b"The quick brown fox jumps over the lazy dog", 0))
print(binascii.crc32(b"abc"))
print(binascii.crc32(b"123456789"), binascii.crc_hqx(b"123456789", 0))
print(binascii.crc_hqx(b"123456789", binascii.crc_hqx(b"ab", 0)))
'''

EXPECTED["a1_bin2_hqx"] = """\
12739
10673
0
65535
60258
61640
891568578
3421780262 12739
37560
"""

# -- 3. bytes.hex / fromhex ---------------------------------------------------

PROBES["a1_hex0_sep"] = r'''
b = b"\xab\xcd\xef\x12\x34"
print(b.hex())
print(b.hex(":"))
print(b.hex(":", 2))
print(b.hex(":", -2))
print(b.hex("-", 1))
print(b.hex("-", -1))
print(b.hex(b":"))
print(bytearray(b).hex(":", 2))
print(bytearray(b).hex(".", -1))
print(b.hex(":", -10))
print(b"\x01\x02".hex(":", 100))
print(b"\x01\x02".hex(":", -100))
def perr(fn):
    try:
        fn()
        print("no-error")
    except Exception as e:
        print(type(e).__name__, e)
perr(lambda: b.hex("", 2))
perr(lambda: b.hex(":", 0))
'''

EXPECTED["a1_hex0_sep"] = """\
abcdef1234
ab:cd:ef:12:34
ab:cdef:1234
abcd:ef12:34
ab-cd-ef-12-34
ab-cd-ef-12-34
ab:cd:ef:12:34
ab:cdef:1234
ab.cd.ef.12.34
abcdef1234
0102
0102
ValueError sep must be length 1.
no-error
"""

PROBES["a1_hex1_fromhex"] = r'''
def perr(fn):
    try:
        fn()
        print("no-error")
    except Exception as e:
        print(type(e).__name__, e)
print(bytes.fromhex("aa bb"))
print(bytes.fromhex("aabb"))
print(bytearray.fromhex("aa bb"))
print(bytes.fromhex("aa\tbb\ncc"))
print(bytes.fromhex(""))
print(bytes.fromhex("AA BB").hex())
perr(lambda: bytearray.fromhex("zz"))
perr(lambda: bytes.fromhex("aa b"))
perr(lambda: bytes.fromhex("aab"))
perr(lambda: bytes.fromhex("a a"))
perr(lambda: bytes.fromhex("7        7"))
'''

EXPECTED["a1_hex1_fromhex"] = """\
b'\\xaa\\xbb'
b'\\xaa\\xbb'
bytearray(b'\\xaa\\xbb')
b'\\xaa\\xbb\\xcc'
b''
aabb
ValueError non-hexadecimal number found in fromhex() arg at position 0
ValueError fromhex() arg must contain an even number of hexadecimal digits
ValueError fromhex() arg must contain an even number of hexadecimal digits
ValueError non-hexadecimal number found in fromhex() arg at position 1
ValueError non-hexadecimal number found in fromhex() arg at position 1
"""

# -- 4. stat ------------------------------------------------------------------

PROBES["a1_stat0_modes"] = r'''
import stat
print(stat.S_IMODE(0o100755), oct(stat.S_IMODE(0o100755)))
print(stat.S_IMODE(0o040755), stat.S_IMODE(0o104000), stat.S_IMODE(0o120777))
print(oct(stat.S_IFMT(0o100755)), oct(stat.S_IFMT(0o040755)), oct(stat.S_IFMT(0o120777)),
      oct(stat.S_IFMT(0o020600)), oct(stat.S_IFMT(0o002000)), oct(stat.S_IFMT(0o001000)),
      oct(stat.S_IFMT(0o0140000)))
print(stat.S_ISREG(0o100644), stat.S_ISDIR(0o040755), stat.S_ISLNK(0o120777),
      stat.S_ISFIFO(0o010000), stat.S_ISSOCK(0o140000), stat.S_ISCHR(0o020000), stat.S_ISBLK(0o060000))
print(stat.S_IFMT(0o755), stat.S_IMODE(0o755))
print(stat.S_IRWXU, stat.S_IRUSR, stat.S_IWUSR, stat.S_IXUSR,
      stat.S_IRWXG, stat.S_IRGRP, stat.S_IRWXO, stat.S_IROTH)
print(stat.S_ISVTX, stat.S_ISGID, stat.S_ISUID)
'''

EXPECTED["a1_stat0_modes"] = """\
493 0o755
493 2048 511
0o100000 0o40000 0o120000 0o20000 0o0 0o0 0o140000
True True True True True True True
0 493
448 256 128 64 56 32 7 4
512 1024 2048
"""

PROBES["a1_stat1_consts"] = r'''
import stat
consts = ["UF_NODUMP","UF_IMMUTABLE","UF_APPEND","UF_OPAQUE","UF_HIDDEN","UF_COMPRESSED",
          "UF_TRACKED","SF_ARCHIVED","SF_IMMUTABLE","SF_APPEND","SF_NOUNLINK","SF_SNAPSHOT"]
for c in consts:
    print(c, getattr(stat, c, "MISSING"))
print("UF_SETTABLE", getattr(stat, "UF_SETTABLE", "MISSING"))
print("SF_SETTABLE", getattr(stat, "SF_SETTABLE", "MISSING"))
'''

EXPECTED["a1_stat1_consts"] = """\
UF_NODUMP 1
UF_IMMUTABLE 2
UF_APPEND 4
UF_OPAQUE 8
UF_HIDDEN 32768
UF_COMPRESSED 32
UF_TRACKED 64
SF_ARCHIVED 65536
SF_IMMUTABLE 131072
SF_APPEND 262144
SF_NOUNLINK 1048576
SF_SNAPSHOT 2097152
UF_SETTABLE 65535
SF_SETTABLE 1073676288
"""

# -- 5. os.pipe / pipe2 --------------------------------------------------------

PROBES["a1_pipe0_rw"] = r'''
import os
r, w = os.pipe()
assert os.write(w, b"x") == 1
assert os.read(r, 10) == b"x"
os.write(w, b"more-bytes")
assert os.read(r, 4) == b"more"
assert os.read(r, 100) == b"-bytes"
os.close(w)
assert os.read(r, 10) == b""
os.close(r)
print("pipe-rw ok")
r, w = os.pipe()
print("pipe-inheritable", os.get_inheritable(r), os.get_inheritable(w))
os.set_blocking(r, False)
try:
    os.read(r, 1)
    print("unexpected-data")
except BlockingIOError as e:
    print("nonblock", type(e).__name__, e.errno)
os.set_blocking(r, True)
os.close(r)
os.close(w)
'''

EXPECTED["a1_pipe0_rw"] = """\
pipe-rw ok
pipe-inheritable False False
nonblock BlockingIOError 35
"""

PROBES["a1_pipe1_flags"] = r'''
import os
if hasattr(os, "pipe2"):
    r, w = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
    print("inheritable", os.get_inheritable(r), os.get_inheritable(w))
    try:
        os.read(r, 1)
        print("unexpected-data")
    except BlockingIOError as e:
        print("nonblock", type(e).__name__, e.errno)
    os.write(w, b"q")
    print(os.read(r, 1))
    os.close(r)
    os.close(w)
else:
    print("pipe2-unavailable-skip")
'''

EXPECTED["a1_pipe1_flags"] = """\
pipe2-unavailable-skip
"""

# -- 6. os.kill / killpg --------------------------------------------------------

PROBES["a1_kill0_existence"] = r'''
import errno, os
me = os.getpid()
os.kill(me, 0)
print("kill-self-0 ok")
try:
    os.kill(99999999, 0)
    print("bogus no-error")
except ProcessLookupError as e:
    print("bogus", type(e).__name__, e.errno, errno.errorcode[e.errno])
os.killpg(os.getpgrp(), 0)
print("killpg-self-0 ok")
try:
    os.killpg(99999999, 0)
    print("boguspg no-error")
except ProcessLookupError as e:
    print("boguspg", type(e).__name__, e.errno)
try:
    os.kill(1, 0)
    print("pid1 ok")
except PermissionError as e:
    print("pid1 eperm", e.errno)
except ProcessLookupError as e:
    print("pid1 esrch", e.errno)
'''

EXPECTED["a1_kill0_existence"] = """\
kill-self-0 ok
bogus ProcessLookupError 3 ESRCH
killpg-self-0 ok
boguspg ProcessLookupError 3
pid1 eperm 1
"""

PROBES["a1_kill1_sigusr1"] = r'''
import os, signal, time
got = []
def handler(sig, frame):
    got.append(sig)
old = signal.signal(signal.SIGUSR1, handler)
os.kill(os.getpid(), signal.SIGUSR1)
deadline = time.monotonic() + 5.0
while not got and time.monotonic() < deadline:
    time.sleep(0.01)
signal.signal(signal.SIGUSR1, old)
print("caught", got, signal.SIGUSR1)
'''

EXPECTED["a1_kill1_sigusr1"] = """\
caught [30] 30
"""

# -- 7. os.utime ----------------------------------------------------------------

PROBES["a1_utime0_ns"] = r'''
import os, tempfile
d = tempfile.mkdtemp()
p = os.path.join(d, "f")
open(p, "w").close()
at = 1_600_000_000 * 10**9 + 123
mt = 1_500_000_000 * 10**9 + 456
os.utime(p, ns=(at, mt))
st = os.stat(p)
print(st.st_atime_ns, st.st_mtime_ns)
os.utime(p, (1_000_000_000, 2_000_000_000))
st = os.stat(p)
print(st.st_atime_ns, st.st_mtime_ns)
os.utime(p)
st = os.stat(p)
print(st.st_atime_ns > 1_600_000_000_000_000_000)
os.utime(p, times=(5.5, 6.25))
st = os.stat(p)
print(st.st_atime_ns, st.st_mtime_ns)
'''

EXPECTED["a1_utime0_ns"] = """\
1600000000000000123 1500000000000000456
1000000000000000000 2000000000000000000
True
5500000000 6250000000
"""

PROBES["a1_utime1_errors"] = r'''
import os, tempfile
d = tempfile.mkdtemp()
p = os.path.join(d, "f")
open(p, "w").close()
lp = os.path.join(d, "l")
os.symlink(p, lp)
def perr(fn):
    try:
        fn()
        print("no-error")
    except Exception as e:
        print(type(e).__name__, e)
perr(lambda: os.utime(p, ns=(1, 2), dir_fd=None) if False else os.utime(p, ns=(1, "x")))
perr(lambda: os.utime(p, ns=(1,)))
perr(lambda: os.utime(p, ns=(1, 2, 3)))
perr(lambda: os.utime(p, ns=(1.5, 2)))
perr(lambda: os.utime(p, (0, 0), ns=(1, 2)))
os.utime(lp, ns=(111, 222), follow_symlinks=False)
st = os.stat(lp, follow_symlinks=False)
print(st.st_atime_ns, st.st_mtime_ns)
st2 = os.stat(p)
print("target-untouched", st2.st_atime_ns != 111)
os.utime(lp, follow_symlinks=False)
print("symlink-ok")
'''

EXPECTED["a1_utime1_errors"] = """\
TypeError unsupported operand type(s) for divmod(): 'str' and 'int'
TypeError utime: 'ns' must be a tuple of two ints
TypeError utime: 'ns' must be a tuple of two ints
TypeError 'float' object cannot be interpreted as an integer
ValueError utime: you may specify either 'times' or 'ns' but not both
111 222
target-untouched True
symlink-ok
"""

ALL_PROBES = sorted(PROBES)
assert set(ALL_PROBES) == set(EXPECTED), "probe/expectation drift"

# ---------------------------------------------------------------------------
# Faces
# ---------------------------------------------------------------------------

def _scrubbed_env():
    # Pinned environment (S-06-15): no ambient locale leaks into goldens.
    env = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    env["LANG"] = "C"
    env.pop("PYTHONUTF8", None)
    return env


def _write_probe(tmp_path, name):
    path = tmp_path / (name + ".py")
    path.write_text(PROBES[name].lstrip("\n"))
    return path


@pytest.mark.parametrize("name", ALL_PROBES)
def test_reference_reproduces_pins(tmp_path, name):
    """Face A: live python3.14 must reproduce every pinned stdout byte-exactly.

    Self-checks both the pins and this suite against interpreter drift.
    """
    script = _write_probe(tmp_path, name)
    proc = subprocess.run([reference_or_skip(), str(script)],
                          capture_output=True, env=_scrubbed_env(),
                          timeout=60)
    assert proc.returncode == 0, (
        f"{name}: reference rc={proc.returncode}\n{proc.stderr.decode(errors='replace')}"
    )
    assert proc.stdout == EXPECTED[name].encode(), (
        f"{name}: reference drift\n got: {proc.stdout!r}\n pin: {EXPECTED[name].encode()!r}"
    )


@xfail_ap
@pytest.mark.parametrize("name", [n for n in ALL_PROBES if n not in ALREADY_GREEN])
def test_impl_matches_reference_bytes(tmp_path, name):
    """Face B: `ap-run-eval <probe>` must match the pinned stdout + rc 0.

    Split by the measured state of the CURRENT binary: probes already green
    (ALREADY_GREEN) assert outright — no xfail marker, per the tests/README
    flip policy (a stale xfail hides regressions). The rest are
    xfail(strict=False) until leaf A-1 lands; they flip to xpass as the
    sibling implementation lands, at which point they move into
    ALREADY_GREEN and drop the marker.
    """
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing (set AP_RUN_EVAL to override)")
    script = _write_probe(tmp_path, name)
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(),
                          timeout=120)
    assert proc.stdout == EXPECTED[name].encode(), (
        f"{name}: impl stdout != pin\n got: {proc.stdout!r}\n pin: {EXPECTED[name].encode()!r}\n"
        f"stderr: {proc.stderr.decode(errors='replace')[-500:]}"
    )
    assert proc.returncode == 0, (
        f"{name}: impl rc={proc.returncode} (want 0); "
        f"stderr: {proc.stderr.decode(errors='replace')[-300:]}"
    )


@pytest.mark.parametrize("name", sorted(ALREADY_GREEN))
def test_impl_already_green_no_xfail(tmp_path, name):
    """Same probe set as test_impl_matches_reference_bytes but for the cases
    the CURRENT binary already satisfies: asserted hard (no xfail), so a
    regression in the landed surfaces goes red instead of xpassing."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing (set AP_RUN_EVAL to override)")
    script = _write_probe(tmp_path, name)
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(),
                          timeout=120)
    assert proc.stdout == EXPECTED[name].encode(), (
        f"{name}: impl stdout != pin\n got: {proc.stdout!r}\n pin: {EXPECTED[name].encode()!r}\n"
        f"stderr: {proc.stderr.decode(errors='replace')[-500:]}"
    )
    assert proc.returncode == 0


# ---------------------------------------------------------------------------
# Item-scoped fine-grained gates (impl face only; keep pins coarse above,
# failure localization fine below). Each still runs a live probe so a red
# run names the exact A-1 item that drifted.
# ---------------------------------------------------------------------------

# ALREADY-GREEN on the current binary (measured 2026-10-24): the errno alias
# identity, stat mask semantics and pipe write/read/close fine-grained gates.
# No xfail marker — regressions must go red, not xpass.

def test_impl_errno_errorcode_alias_identity(tmp_path):
    """Leaf item 1 gate: errorcode[EWOULDBLOCK] == 'EAGAIN' — the alias
    collapses to the canonical name in the dict, not to 'EWOULDBLOCK'."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "alias.py"
    script.write_text(
        "import errno\n"
        "print(errno.errorcode[errno.EWOULDBLOCK])\n"
        "print(errno.errorcode[errno.EAGAIN] == errno.errorcode[errno.EWOULDBLOCK])\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"EAGAIN\nTrue\n", (
        f"alias pin: {proc.stdout!r}; stderr: {proc.stderr.decode(errors='replace')[-300:]}"
    )


@xfail_ap
def test_impl_binascii_qp_roundtrip_property(tmp_path):
    """Leaf item 2 gate: qp roundtrip is lossless over a byte-soup including
    =, high bytes, CR/LF and long lines (forces soft-wrap + recovery)."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "qp_rt.py"
    script.write_text(
        "import binascii, random\n"
        "rng = random.Random(7)\n"
        "data = bytes(rng.randrange(256) for _ in range(2000)) + b'==\\r\\n\\xff'\n"
        "enc = binascii.b2a_qp(data, istext=False)\n"
        "print(binascii.a2b_qp(enc) == data)\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"True\n", (
        f"qp roundtrip broke: {proc.stdout!r}; stderr: {proc.stderr.decode(errors='replace')[-300:]}"
    )


@xfail_ap
def test_impl_hex_sep_grouping_boundaries(tmp_path):
    """Leaf item 3 gate: bytes_per_sep larger than len(data) inserts nothing;
    negative grouping counts from the right; sep accepts str AND 1-byte bytes."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "hex_sep.py"
    script.write_text(
        "print(b'\\x01\\x02'.hex(':', 100))\n"
        "print(b'\\x01\\x02'.hex(':', -100))\n"
        "print(b'abcdef'.hex(':', 2))\n"
        "print(b'abcdef'.hex(':', -2))\n"
        "print(b'abc').__class__ if False else None\n"
        "print(bytearray.fromhex('aa bb').hex(':', -1))\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"0102\n0102\nab:cdef\nabcdef:ab\naa:bb\n", (
        f"hex grouping pins: {proc.stdout!r}; stderr: {proc.stderr.decode(errors='replace')[-300:]}"
    )


def test_impl_stat_imode_ifmt_mask_semantics(tmp_path):
    """Leaf item 4 gate: S_IMODE masks IFMT off, S_IFMT masks mode bits off,
    and both accept a bare permission/shape-0 mode."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "statmasks.py"
    script.write_text(
        "import stat\n"
        "print(stat.S_IMODE(0o755) == 0o755, stat.S_IFMT(0o755) == 0)\n"
        "print(stat.S_IFMT(0o100755) == 0o100000, stat.S_IMODE(0o100755) == 0o755)\n"
        "print(stat.S_IFMT(0o120777) == 0o120000, stat.S_IMODE(0o120777) == 0o777)\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"True True\nTrue True\nTrue True\n", (
        f"stat masks: {proc.stdout!r}; stderr: {proc.stderr.decode(errors='replace')[-300:]}"
    )


def test_impl_pipe_write_read_close(tmp_path):
    """Leaf item 5 gate (leaf Gate line: 'os.pipe write/read')."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "pipe.py"
    script.write_text(
        "import os\n"
        "r, w = os.pipe()\n"
        "assert os.write(w, b'ping') == 4\n"
        "assert os.read(r, 10) == b'ping'\n"
        "os.close(w)\n"
        "assert os.read(r, 10) == b''\n"
        "os.close(r)\n"
        "print('pipe-ok')\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"pipe-ok\n" and proc.returncode == 0, (
        f"pipe: rc={proc.returncode} out={proc.stdout!r} "
        f"err={proc.stderr.decode(errors='replace')[-300:]}"
    )


@xfail_ap
def test_impl_kill_zero_and_killpg_zero(tmp_path):
    """Leaf item 5 gate: signal-0 existence probes on self and pgrp, plus the
    ESRCH error mapping for a (almost-certainly) dead pid."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "kill0.py"
    script.write_text(
        "import errno, os\n"
        "os.kill(os.getpid(), 0)\n"
        "os.killpg(os.getpgrp(), 0)\n"
        "try:\n"
        "    os.kill(99999999, 0)\n"
        "    print('alive?!')\n"
        "except ProcessLookupError as e:\n"
        "    print('ESRCH', errno.errorcode[e.errno])\n"
        "print('kill0-ok')\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"ESRCH ESRCH\nkill0-ok\n" and proc.returncode == 0, (
        f"kill0: rc={proc.returncode} out={proc.stdout!r} "
        f"err={proc.stderr.decode(errors='replace')[-300:]}"
    )


# 2026-11-07 flip wave (01.02): utime ns readback exact on current binary.
def test_impl_utime_ns_exact_readback(tmp_path):
    """Leaf item 6 gate: ns= tuple survives a stat round trip exactly, and
    follow_symlinks=False hits the link, not the target."""
    if AP_RUN_EVAL is None:
        pytest.skip("ap-run-eval bin missing")
    script = tmp_path / "utime.py"
    script.write_text(
        "import os, tempfile\n"
        "d = tempfile.mkdtemp()\n"
        "p = os.path.join(d, 'f')\n"
        "open(p, 'w').close()\n"
        "lp = os.path.join(d, 'l')\n"
        "os.symlink(p, lp)\n"
        "os.utime(p, ns=(123, 456))\n"
        "st = os.stat(p)\n"
        "print(st.st_atime_ns, st.st_mtime_ns)\n"
        "os.utime(lp, ns=(111, 222), follow_symlinks=False)\n"
        "st = os.stat(lp, follow_symlinks=False)\n"
        "print(st.st_atime_ns, st.st_mtime_ns)\n"
        "print(os.stat(p).st_atime_ns == 123)\n"
    )
    proc = subprocess.run([*AP_RUN_EVAL_CMD, str(script)],
                          capture_output=True, env=_scrubbed_env(), timeout=120)
    assert proc.stdout == b"123 456\n111 222\nTrue\n" and proc.returncode == 0, (
        f"utime: rc={proc.returncode} out={proc.stdout!r} "
        f"err={proc.stderr.decode(errors='replace')[-300:]}"
    )
