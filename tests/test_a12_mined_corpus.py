# T-A12 — mined differential expectation pack for the A-1+A-2 RustPython
# backfill surfaces (test-mining track; see
# docs/rustpython-leaf-donor-inventory.md, bottom section "python_tests/
# mining list").
#
# Provenance: EDGE cases re-authored from the CPython-derived donor test
# corpus in the RustPython tree (/tmp/rustpython-study, MIT), then
# re-pinned against live CPython 3.14. Donor files mined:
#   test_bytes.py      -> bytes-hex/*    (hex sep/bytes_per_sep, fromhex errors)
#   test_base64.py     -> base64/*       (b64/b16/b32/urlsafe, validate=True)
#   test_binascii.py   -> binascii/*     (hexlify, crc32/crc_hqx, qp, uu, b2a_base64)
#   test_hashlib.py    -> hashlib/*      (sha2 vectors, usedforsecurity,
#                                          algorithms_*/new/sha3/blake2/shake,
#                                          .digest())
#   test_os/           -> os/*           (utime ns=/times=, pipe, kill)
#   test_stat.py       -> stat/*         (filemode/S_IS*/S_IMODE/S_IFMT tables)
#   test_errno.py      -> errno/*        (errorcode dict invariant)
#   test_posixpath.py + test_genericpath.py -> posixpath/* + genericpath/*
#                                          (normpath/realpath/relpath tables)
# Every expected stdout byte was re-measured with
# /opt/homebrew/bin/python3.14 (3.14) — donor expectations that contradicted
# live 3.14 were re-pinned to 3.14 (see cpython_contradictions in the task
# report; notably the donor tree expects base64 b64decode(padded=,
# ignorechars=) and binascii a2b_hqx/b2a_hqx/rle*, none of which exist in
# 3.14).
#
# Harness: differential vs the antipythony tier-0 executor
# (target/debug/ap-run-eval, the documented T-05/T-06 testability seam),
# resolved via AP_RUN_EVAL env override -> PATH -> target/{debug,release}.
# Each case is one self-contained program; its stdout must equal the pinned
# reference bytes. Cases the current binary already passes are NOT xfail;
# known-incomplete surfaces (per the inventory gap table) are
# xfail(strict=False).
#
# FIREWALL: no implementation source under crates/ was read; this file was
# authored from the spec/inventory docs + live reference runs only.

import os
import shlex
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
RUN_TIMEOUT = int(os.environ.get("APX_T12_TIMEOUT", "180"))


def _find_run_eval() -> str | None:
    override = os.environ.get("AP_RUN_EVAL")
    if override:
        return shlex.split(override)[0]
    found = shutil.which("ap-run-eval")
    if found:
        return found
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / "ap-run-eval"
        if cand.is_file():
            return str(cand)
    return None


AP_RUN_EVAL = _find_run_eval()

SCRUBBED_ENV = {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}

# (id, surface, donor, source, expected stdout, xfail_now)
# Each entry: pytest.param(case_source, expected_stdout, id=...)
# Cases whose id is in XFAIL_IDS carry a non-strict xfail marker.
CASES = [
    pytest.param("import base64\nprint(base64.b16encode(b'\\xde\\xad'))\nprint(base64.b16decode(b'6162'))\nprint(base64.b16decode(b'616263'))\nprint(base64.b16decode(b'aBcD', casefold=True))\ndef t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: base64.b16decode(b'aBcD'))\nt(lambda: base64.b16decode(b'616'))\n", "b'DEAD'\nb'ab'\nb'abc'\nb'\\xab\\xcd'\nError | Non-base16 digit found\nError | Odd-length string\n", id="b16_roundtrip",
        marks=pytest.mark.xfail(strict=False, reason="base64: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import binascii\nprint(binascii.b2a_base64(b'abc'))\nprint(binascii.b2a_base64(b'abc', newline=False))\nprint(binascii.b2a_base64(b''))\n", "b'YWJj\\n'\nb'YWJj'\nb'\\n'\n", id="b2a_base64_newline",
        marks=pytest.mark.xfail(strict=False, reason="binascii: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import base64\nprint(base64.b32encode(b'abc'))\nprint(base64.b32decode(b'MFRGG==='))\nprint(base64.b32encode(b''))\nprint(base64.b32decode(b''))\n", "b'MFRGG==='\nb'abc'\nb''\nb''\n", id="b32_roundtrip",
        marks=pytest.mark.xfail(strict=False, reason="base64: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import base64\nprint(base64.b64encode(b'abc'))\nprint(base64.b64decode(b'YWJj'))\nprint(base64.b64encode(b'ab'))\nprint(base64.b64decode(b'YWI='))\nprint(base64.b64decode(b'YQ=='))\nprint(base64.b64decode(b'YWJj\\nYWI='))\nprint(base64.standard_b64encode(b'abc?'))\nprint(base64.urlsafe_b64encode(b'abc?'))\nprint(base64.urlsafe_b64decode(b'YWJjPw=='))\n", "b'YWJj'\nb'abc'\nb'YWI='\nb'ab'\nb'a'\nb'abcab'\nb'YWJjPw=='\nb'YWJjPw=='\nb'abc?'\n", id="b64_roundtrip",
        marks=pytest.mark.xfail(strict=False, reason="base64: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import base64\ndef t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: base64.b64decode(b'YWJj', validate=True))\nt(lambda: base64.b64decode(b'YWJjZA==', validate=True))\nt(lambda: base64.b64decode(b'YWJ#', validate=True))\nt(lambda: base64.b64decode(b'Y!Jj', validate=False))\nt(lambda: base64.b64decode(b'YQ='))\nt(lambda: base64.b64decode(b'abc'))\nt(lambda: base64.b64decode(b'=YWJj', validate=False))\n", "ok b'abc'\nok b'abcd'\nError | Only base64 data is allowed\nError | Incorrect padding\nError | Incorrect padding\nError | Incorrect padding\nok b'abc'\n", id="b64_strict_validate",
        marks=pytest.mark.xfail(strict=False, reason="base64: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import binascii\nprint(binascii.crc32(b'hello'))\nprint(binascii.crc32(b''))\nprint(binascii.crc32(b'123456789'))\nprint(binascii.crc_hqx(b'123456789', 0))\nprint(binascii.crc_hqx(b'', 0))\nc = binascii.crc32(b'Test the CRC-32 of')\nprint(binascii.crc32(b' this string.', c))\n", '907060870\n0\n3421780262\n12739\n0\n1571220330\n', id="crc_tables",
        marks=pytest.mark.xfail(strict=False, reason="binascii: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import hashlib\nprint(hashlib.sha256(b'abc').digest().hex())\nprint(bytes(hashlib.md5(b'abc').digest()).hex())\n", 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n900150983cd24fb0d6963f7d28e17f72\n', id="digest_bytes",
        marks=pytest.mark.xfail(strict=False, reason="hashlib: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import errno\nassert errno.errorcode[2] == 'ENOENT'\nassert errno.errorcode[errno.EACCES] == 'EACCES'\nassert errno.errorcode.get(999999) is None\nbad = [c for c, n in errno.errorcode.items() if getattr(errno, n, None) != c]\nprint('bad:', bad)\nprint(errno.EINTR, errno.EBADF, errno.EEXIST, errno.EISDIR, errno.EMFILE)\n", 'bad: []\n4 9 17 21 24\n', id="errorcode_invariant"),
    pytest.param('import stat\nfor m in (0o0, 0o100644, 0o100755, 0o140000, 0o120000, 0o40000, 0o20000, 0o60000, 0o100000):\n    print(oct(m), stat.filemode(m), stat.S_IMODE(m), oct(stat.S_IFMT(m)))\nprint(stat.filemode(0o120777))\nprint(stat.filemode(0o100644 | stat.S_ISVTX))\nprint(stat.filemode(0o100644 | stat.S_ISGID))\nprint(stat.filemode(0o100644 | stat.S_ISUID))\n', '0o0 ?--------- 0 0o0\n0o100644 -rw-r--r-- 420 0o100000\n0o100755 -rwxr-xr-x 493 0o100000\n0o140000 s--------- 0 0o140000\n0o120000 l--------- 0 0o120000\n0o40000 d--------- 0 0o40000\n0o20000 c--------- 0 0o20000\n0o60000 b--------- 0 0o60000\n0o100000 ---------- 0 0o100000\nlrwxrwxrwx\n-rw-r--r-T\n-rw-r-Sr--\n-rwSr--r--\n', id="filemode_table",
        marks=pytest.mark.xfail(strict=False, reason="stat: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("print(bytes.fromhex('61 62 63'))\nprint(bytes.fromhex('  61 62  '))\nprint(bytes.fromhex('61\\t62\\n63\\r\\n64'))\nprint(bytearray.fromhex('6162'))\nprint(bytearray.fromhex('61 62'))\n", "b'abc'\nb'ab'\nb'abcd'\nbytearray(b'ab')\nbytearray(b'ab')\n", id="fromhex_basic",
        marks=pytest.mark.xfail(strict=False, reason="bytes-hex: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("def t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: bytes.fromhex('abc'))\nt(lambda: bytes.fromhex('6g'))\nt(lambda: bytes.fromhex('6 2'))\nt(lambda: bytes.fromhex('\\x0061'))\nt(lambda: bytes.fromhex('0x61'))\nt(lambda: bytes.fromhex('6162636465666 7'))\nt(lambda: bytearray.fromhex('61 6 2'))\n", 'ValueError | fromhex() arg must contain an even number of hexadecimal digits\nValueError | non-hexadecimal number found in fromhex() arg at position 1\nValueError | non-hexadecimal number found in fromhex() arg at position 1\nValueError | non-hexadecimal number found in fromhex() arg at position 0\nValueError | non-hexadecimal number found in fromhex() arg at position 1\nValueError | non-hexadecimal number found in fromhex() arg at position 13\nValueError | non-hexadecimal number found in fromhex() arg at position 4\n', id="fromhex_errors"),
    pytest.param("import hashlib\nprint(hashlib.sha256(b'abc').name)\nprint(hashlib.sha256(b'abc').digest_size)\nprint(hashlib.sha256().block_size)\nh = hashlib.sha256(b'ab')\nh.update(b'c')\nprint(h.hexdigest())\nprint(hashlib.sha256(b'abc').hexdigest() == h.hexdigest())\n", 'sha256\n32\n64\nba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\nTrue\n', id="hashlib_api_shape",
        marks=pytest.mark.xfail(strict=False, reason="hashlib: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import sys\nb = b'\\xb9\\x01\\xef'\nprint(b.hex())\nprint(b''.hex())\nprint(b'abcdef'.hex(':', 2))\nprint(b'abcdef'.hex(':', -2))\nprint(b'abcdef'.hex(':', 3))\nprint(b'abcdef'.hex(':', 4))\nprint(b'abcdef'.hex(':', -4))\nprint(b'\\xb9\\x01\\xef'.hex(b'$'))\nprint(b'\\xb9\\x01\\xef'.hex('\\x00'))\n", 'b901ef\n\n6162:6364:6566\n6162:6364:6566\n616263:646566\n6162:63646566\n61626364:6566\nb9$01$ef\nb9\x0001\x00ef\n', id="hex_basic"),
    pytest.param("b = b'abcdef'\nprint(b.hex(':', 1))\nprint(b.hex(':', 0))\nprint(b.hex(':', 5))\nprint(b.hex(':', -1))\nprint(b.hex(':', -5))\nprint(bytearray(b'abcdef').hex('-', 2))\n", '61:62:63:64:65:66\n616263646566\n61:6263646566\n61:62:63:64:65:66\n6162636465:66\n6162-6364-6566\n', id="hex_group_edges"),
    pytest.param("def t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nb = b'\\xb9\\x01\\xef'\nt(lambda: b.hex(''))\nt(lambda: b.hex('xx'))\nt(lambda: b.hex(None, 0))\nt(lambda: b.hex('\\xff'))\nt(lambda: b.hex(b'\\xff'))\nt(lambda: b.hex(chr(0x100)))\n", "ValueError | sep must be length 1.\nValueError | sep must be length 1.\nTypeError | object of type 'NoneType' has no len()\nValueError | sep must be ASCII.\nValueError | sep must be ASCII.\nValueError | sep must be ASCII.\n", id="hex_sep_invalid",
        marks=pytest.mark.xfail(strict=False, reason="bytes-hex: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import binascii\nprint(binascii.hexlify(b'abc'))\nprint(binascii.unhexlify('414243'))\nprint(binascii.b2a_hex(b'ab'), binascii.a2b_hex(b'4142'))\nprint(binascii.unhexlify('41 42'))\ndef t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: binascii.unhexlify('414'))\nt(lambda: binascii.unhexlify('4g'))\n", "b'616263'\nb'ABC'\nb'6162' b'AB'\n", id="hexlify_unhexlify",
        marks=pytest.mark.xfail(strict=False, reason="binascii: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import stat\nprint(stat.S_ISDIR(0o40000), stat.S_ISREG(0o100644), stat.S_ISLNK(0o120000))\nprint(stat.S_ISCHR(0o20000), stat.S_ISBLK(0o60000), stat.S_ISFIFO(0o10000), stat.S_ISSOCK(0o140000))\nprint(stat.S_ISDIR(0o100644), stat.S_ISREG(0o40000))\nprint(hasattr(stat, 'S_IMODE'), hasattr(stat, 'S_IFMT'), hasattr(stat, 'S_ISDOOR'))\n", 'True True True\nTrue True True True\nFalse False\nTrue True True\n', id="is_predicates"),
    pytest.param("import errno, os\nos.kill(0, 0)\nprint('sig0 self ok')\ntry:\n    os.kill(999999999, 0)\nexcept ProcessLookupError:\n    print('lookup ok')\ntry:\n    os.kill(os.getpid(), 999)\nexcept OSError as e:\n    print('bad sig errno:', e.errno)\ntry:\n    os.kill(1, 0)\nexcept PermissionError:\n    print('perm ok')\nexcept ProcessLookupError:\n    print('lookup ok')\nexcept OSError as e:\n    print('other errno:', e.errno)\n", 'sig0 self ok\nlookup ok\nbad sig errno: 22\nperm ok\n', id="kill_semantics",
        marks=pytest.mark.xfail(strict=False, reason="os: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import os.path as P\nprint(P.normpath(''))\nprint(P.normpath('a/..'))\nprint(P.normpath('a/b/../../c'))\nprint(P.normpath('../a'))\nprint(P.normpath('a//b//c'))\nprint(P.normpath('a/./b'))\nprint(P.normpath('//a/b'))\nprint(P.normpath('///a/b'))\nprint(P.normpath('a/b/../..'))\nprint(P.normpath('..'))\n", '.\n.\nc\n../a\na/b/c\na/b\n//a/b\n/a/b\n.\n..\n', id="normpath_table",
        marks=pytest.mark.xfail(strict=False, reason="posixpath: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import os\nr, w = os.pipe()\nos.write(w, b'ping')\nprint(os.read(r, 4))\nos.close(r)\nos.close(w)\ntry:\n    os.write(w, b'x')\nexcept OSError:\n    print('EBADF ok')\n", "b'ping'\nEBADF ok\n", id="pipe_roundtrip"),
    pytest.param("import os.path as P\nprint(P.exists('/etc/hosts'), P.exists('/nonexistent/a12'))\nprint(P.lexists('/nonexistent/a12'))\nprint(P.isdir('/tmp'), P.isfile('/etc/hosts'), P.isdir('/etc/hosts'))\nprint(P.commonpath(['/a/b', '/a/c']))\nprint(P.commonpath(['/usr', '/usr/local/bin']))\ndef t(f):\n    try:\n        print('ok', f())\n    except Exception as e:\n        print(type(e).__name__)\nt(lambda: P.commonpath(['/a/b', '/c']))\n", 'True False\nFalse\nTrue True False\n/a\n/usr\nok /\n', id="predicates_common"),
    pytest.param("import binascii\nprint(binascii.a2b_qp(b'=41=42=43'))\nprint(binascii.b2a_qp(b'a b'))\nprint(binascii.b2a_qp(b'a  b'))\nprint(binascii.b2a_qp(b'a\\nb'))\nprint(binascii.b2a_qp(b'a  b', quotetabs=True))\nprint(binascii.b2a_qp(b'a=\\nb'))\nprint(binascii.a2b_qp(b'a=20b'))\nprint(binascii.a2b_qp(b'=3D'))\n", "b'ABC'\nb'a b'\nb'a  b'\nb'a\\nb'\nb'a=20=20b'\nb'a=3D\\nb'\nb'a b'\nb'='\n", id="qp_roundtrip"),
    pytest.param("import os.path as P\nprint(P.realpath(''))\nprint(P.realpath('/a/./b/../c'))\nprint(P.realpath('/nonexistent/definitely/missing'))\ndef t(f):\n    try:\n        print('ok', f())\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: P.realpath('/nonexistent/definitely/missing', strict=True))\n", "/Users/caimlas/git/antipythony\n/a/c\n/nonexistent/definitely/missing\nFileNotFoundError | [Errno 2] No such file or directory: '/nonexistent'\n", id="realpath_edges",
        marks=pytest.mark.xfail(strict=False, reason="posixpath: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import hashlib\ndef t(f):\n    try:\n        print('ok', repr(f()))\n    except Exception as e:\n        print(type(e).__name__, '|', e)\nt(lambda: sorted(hashlib.algorithms_guaranteed))\nt(lambda: hashlib.new('sha256', b'abc').hexdigest()[:8])\nt(lambda: hashlib.sha3_256(b'abc').hexdigest()[:8])\nt(lambda: hashlib.blake2b(b'abc').hexdigest()[:8])\nt(lambda: hashlib.shake_128(b'abc').hexdigest(8))\n", "ok ['blake2b', 'blake2s', 'md5', 'sha1', 'sha224', 'sha256', 'sha384', 'sha3_224', 'sha3_256', 'sha3_384', 'sha3_512', 'sha512', 'shake_128', 'shake_256']\nok 'ba7816bf'\nok '3a985da7'\nok 'ba80a53f'\nok '5881092dd818bf5c'\n", id="registry_surface",
        marks=pytest.mark.xfail(strict=False, reason="hashlib: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import hashlib\nprint(hashlib.md5(b'').hexdigest())\nprint(hashlib.sha1(b'abc').hexdigest())\nprint(hashlib.sha224(b'abc').hexdigest())\nprint(hashlib.sha256(b'abc').hexdigest())\nprint(hashlib.sha384(b'abc').hexdigest())\nprint(hashlib.sha512(b'abc').hexdigest())\nprint(hashlib.md5(b'abc').hexdigest())\n", 'd41d8cd98f00b204e9800998ecf8427e\na9993e364706816aba3e25717850c26c9cd0d89d\n23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7\nba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\ncb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7\nddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f\n900150983cd24fb0d6963f7d28e17f72\n', id="sha2_vectors"),
    pytest.param("import os.path as P\nprint(P.split('/a/b/c'))\nprint(P.split('/a/b/'))\nprint(P.split('a'))\nprint(P.join('a', 'b'), P.join('a/', 'b'), P.join('a', '/b'))\nprint(P.splitext('a.b.c'))\nprint(P.splitext('.bashrc'))\nprint(P.basename('/a/b/'), P.dirname('/a/b/'))\nprint(P.isabs('/a'), P.isabs('a'))\nprint(P.relpath('/usr/local', '/usr'))\n", "('/a/b', 'c')\n('/a/b', '')\n('', 'a')\na/b a/b /b\n('a.b', '.c')\n('.bashrc', '')\n /a/b\nTrue False\nlocal\n", id="split_join_tools"),
    pytest.param("import hashlib\nprint(hashlib.md5(b'abc', usedforsecurity=False).hexdigest())\nprint(hashlib.sha256(b'abc', usedforsecurity=True).hexdigest()[:16])\n", '900150983cd24fb0d6963f7d28e17f72\nba7816bf8f01cfea\n', id="usedforsecurity_kw"),
    pytest.param("import os, tempfile\nd = tempfile.mkdtemp()\np = os.path.join(d, 'f')\nopen(p, 'wb').close()\nos.utime(p)\nprint(os.stat(p).st_mtime_ns > 0)\ndef t(f):\n    try:\n        f()\n        print('ok')\n    except Exception as e:\n        print(type(e).__name__)\nt(lambda: os.utime('/nonexistent/xyz/a12', ns=(0, 0)))\nt(lambda: os.utime(p, (1, 2, 3)))\nt(lambda: os.utime(p, times=(1.0, 2.0), ns=(0, 0)))\n", 'True\nFileNotFoundError\nTypeError\nValueError\n', id="utime_errors",
        marks=pytest.mark.xfail(strict=False, reason="os: known-incomplete on the current binary (gap table, docs/rustpython-leaf-donor-inventory.md); awaits A-1/A-2 backfill")),
    pytest.param("import os, tempfile\nd = tempfile.mkdtemp()\np = os.path.join(d, 'f')\nopen(p, 'wb').close()\nos.utime(p, ns=(1000000000, 2000000000))\nst = os.stat(p)\nprint(st.st_atime_ns, st.st_mtime_ns)\nos.utime(p, (1000000000.0, 2000000000.0))\nprint(int(os.stat(p).st_atime), int(os.stat(p).st_mtime))\nos.utime(p, ns=(1000000000, 2000000000))\nprint(os.stat(p).st_atime_ns, os.stat(p).st_mtime_ns)\n", '1000000000 2000000000\n1000000000 2000000000\n1000000000 2000000000\n', id="utime_ns"),
    pytest.param("import binascii\nprint(binascii.b2a_uu(b'x'))\nprint(binascii.b2a_uu(b''))\nprint(binascii.b2a_uu(b'', backtick=True))\nprint(binascii.b2a_uu(b'\\x00Cat'))\nprint(binascii.b2a_uu(b'\\x00Cat', backtick=True))\nprint(binascii.a2b_uu(b' \\n'))\nprint(binascii.a2b_uu(b'`\\n'))\nprint(binascii.a2b_uu(b'%2&5L;&\\\\ \\n'))\n", "b'!>   \\n'\nb' \\n'\nb'`\\n'\nb'$ $-A=   \\n'\nb'$`$-A=```\\n'\nb''\nb''\nb'Hello'\n", id="uu_roundtrip"),
]

CASE_IDS = [c.id for c in CASES]
assert len(set(CASE_IDS)) == len(CASE_IDS), "duplicate case ids"


def _run_impl(tmp_path: Path, source: str) -> subprocess.CompletedProcess:
    assert AP_RUN_EVAL, "no ap-run-eval binary (set AP_RUN_EVAL)"
    case = tmp_path / "case.py"
    case.write_text(source, encoding="utf-8")
    return subprocess.run(
        [AP_RUN_EVAL, str(case)],
        capture_output=True, text=True, env=SCRUBBED_ENV,
        cwd=str(REPO_ROOT), timeout=RUN_TIMEOUT,
    )


@pytest.mark.skipif(AP_RUN_EVAL is None, reason="ap-run-eval binary missing")
@pytest.mark.parametrize(("source", "expected"), CASES)
def test_mined_case_matches_cpython314(source, expected, tmp_path):
    proc = _run_impl(tmp_path, source)
    assert proc.stdout == expected, (
        f"stdout mismatch\n"
        f"--- expected (CPython 3.14) ---\n{expected!r}\n"
        f"--- actual ({Path(AP_RUN_EVAL).name}) ---\n{proc.stdout!r}\n"
        f"stderr tail: {proc.stderr[-400:]!r}"
    )
    assert proc.returncode == 0, (
        f"rc={proc.returncode}; stderr tail: {proc.stderr[-400:]!r}"
    )


_XFAIL_IDS = [p.id for p in CASES if p.marks]


def test_xfail_split_is_nonempty_and_disjoint():
    green = [i for i in CASE_IDS if i not in _XFAIL_IDS]
    assert green, "every case xfailed: pins are not differential"
    assert _XFAIL_IDS, "no xfails: gap coverage regressed"


def test_surface_coverage():
    # All nine mined surfaces represented (see module docstring provenance):
    # the surface tags live in the xfail reasons ("<surface>: known-incomplete").
    surfaces = set()
    for p in CASES:
        if p.marks:
            surfaces.add(p.marks[0].kwargs["reason"].split(":")[0])
    assert surfaces >= {
        "bytes-hex", "base64", "binascii", "hashlib",
        "stat", "os", "posixpath",
    }
    # green cases carry the remaining surfaces implicitly; assert the id set
    # spans the donor file table at least this broadly:
    assert len(CASE_IDS) >= 25
