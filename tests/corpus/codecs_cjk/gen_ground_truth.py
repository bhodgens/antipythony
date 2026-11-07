#!/usr/bin/env python3
"""Generate ground-truth probe data from the CPython 3.14 reference for every
multibyte codec. Writes one JSON file per codec with encode/decode/error cases
as repr'd (ASCII) strings so antipythony can eval() them with ast.literal_eval."""
import json
import sys
import os

REF = "/opt/homebrew/bin/python3.14"

CODECS = [
    # (module, codec getcodec key, wrapper module name, canonical CodecInfo.name)
    ("jp", "cp932", "cp932"),
    ("jp", "euc_jp", "euc_jp"),
    ("jp", "shift_jis", "shift_jis"),
    ("jp", "shift_jisx0213", "shift_jisx0213"),
    ("jp", "shift_jis_2004", "shift_jis_2004"),
    ("jp", "euc_jisx0213", "euc_jisx0213"),
    ("jp", "euc_jis_2004", "euc_jis_2004"),
    ("iso2022", "iso2022_jp", "iso2022_jp"),
    ("iso2022", "iso2022_jp_1", "iso2022_jp_1"),
    ("iso2022", "iso2022_jp_2", "iso2022_jp_2"),
    ("iso2022", "iso2022_jp_3", "iso2022_jp_3"),
    ("iso2022", "iso2022_jp_ext", "iso2022_jp_ext"),
    ("iso2022", "iso2022_jp_2004", "iso2022_jp_2004"),
    ("iso2022", "iso2022_kr", "iso2022_kr"),
    ("kr", "euc_kr", "euc_kr"),
    ("kr", "cp949", "cp949"),
    ("kr", "johab", "johab"),
    ("cn", "gb2312", "gb2312"),
    ("cn", "gbk", "gbk"),
    ("cn", "gb18030", "gb18030"),
    ("cn", "hz", "hz"),
    ("tw", "big5", "big5"),
    ("tw", "cp950", "cp950"),
    ("hk", "big5hkscs", "big5hkscs"),
]

SAMPLES = [
    "hello",
    "café",  # latin-1 in strict; raises in CJK strict
    "\u4e2d\u6587",  # Chinese
    "\u65e5\u672c\u8a9e",  # Japanese
    "\ud55c\uad6d\uc5b4",  # Korean
    "\uff21\uff22",  # fullwidth A B
    "\u00e9\u20ac",
    "\u9ad8\u5fb7\u7eb2",  # GBK-only chars
    "\uac00\uac01",  # Hangul
    "\u203b",  # JIS X 0208 symbol
    "\u30e2\u30fc\u30b7\u30e7\u30f3",
    "\u4e00\u4e01",  # CJK basic
    "\U0002f800",  # CJK ext (big5hkscs/gb18030 4-byte)
    "\u30fb",  # katakana middle dot (iso2022_jp_2/ext special)
    "\u2116",  # numero sign
]

# bytes samples for decode: use roundtrip bytes from encode + crafted invalid
CRAFTED_INVALID = {
    "cp932": b"\x81\x00",
    "euc_jp": b"\x8f\x00",
    "shift_jis": b"\x81\x00",
    "gb2312": b"\xa1\x00",
    "gbk": b"\x81\x00",
    "gb18030": b"\x81",
    "big5": b"\x88\x80",
    "big5hkscs": b"\x88\x80",
    "cp949": b"\x81\x00",
    "euc_kr": b"\x81\x00",
    "johab": b"\x84\x00",
    "hz": b"~\x00",
    "iso2022_jp": b"\x1b$B\x00",
    "iso2022_kr": b"\x1b$)C\x00",
}

out = {}
for _mod, key, wrapper in CODECS:
    entry = {"wrapper": wrapper, "canonical": None, "encode": [], "decode": [], "errors": [], "bom": None}
    code = f"""
import codecs
info = codecs.lookup({key!r})
print(info.name)
"""
    # We run a child for each codec to keep state clean.
    lines = []
    lines.append("import codecs, json")
    lines.append(f"info = codecs.lookup({key!r})")
    lines.append("res = {'canonical': info.name}")
    # encode ground truth
    enc = {}
    dec = {}
    errs = {}
    for samp in SAMPLES:
        tag = "s%d" % len(enc)
        try:
            b = samp.encode(key)
            enc[tag] = [samp, b.hex()]
        except UnicodeEncodeError:
            enc[tag] = [samp, None]  # strict encode fails
            try:
                br = samp.encode(key, "replace")
                enc[tag + "_replace"] = br.hex()
            except Exception:
                pass
            try:
                bi = samp.encode(key, "ignore")
                enc[tag + "_ignore"] = bi.hex()
            except Exception:
                pass
            try:
                bb = samp.encode(key, "backslashreplace")
                enc[tag + "_backslashreplace"] = bb.hex()
            except Exception:
                pass
            try:
                bx = samp.encode(key, "xmlcharrefreplace")
                enc[tag + "_xmlcharrefreplace"] = bx.hex()
            except Exception:
                pass
        # decode the same bytes (if encode succeeded, decode should round-trip)
    for samp in SAMPLES:
        try:
            b = samp.encode(key)
        except UnicodeEncodeError:
            continue
        tag = "d%d" % len(dec)
        try:
            t = b.decode(key)
            dec[tag] = [b.hex(), t, len(samp)]
        except UnicodeDecodeError:
            dec[tag] = [b.hex(), None, len(samp)]
    # replace/ignore decode on crafted invalid bytes
    bad = CRAFTED_INVALID.get(key)
    if bad is not None:
        try:
            r = bad.decode(key, "replace")
            errs["bad_replace"] = [bad.hex(), r]
        except Exception as e:
            errs["bad_replace"] = [bad.hex(), None, type(e).__name__, str(e)[:80]]
        try:
            i2 = bad.decode(key, "ignore")
            errs["bad_ignore"] = [bad.hex(), i2]
        except Exception as e:
            errs["bad_ignore"] = [bad.hex(), None, type(e).__name__, str(e)[:80]]
        try:
            bad.decode(key)  # strict
            errs["bad_strict"] = [bad.hex(), ""]
        except UnicodeDecodeError as e:
            errs["bad_strict"] = [bad.hex(), None, type(e).__name__, str(e)]
        except Exception as e:
            errs["bad_strict"] = [bad.hex(), None, type(e).__name__, str(e)[:80]]
        # strict error position/attrs
        try:
            bad.decode(key)
        except UnicodeDecodeError as e:
            errs["bad_strict_attrs"] = [e.object.hex(), e.start, e.end, e.reason]
    # trailing partial byte (incomplete multibyte at end)
    for samp in SAMPLES:
        try:
            b = samp.encode(key)
        except UnicodeEncodeError:
            continue
        if len(b) > 1:
            partial = b[:-1]
            try:
                t = partial.decode(key)
                errs.setdefault("partials", []).append([partial.hex(), t])
            except UnicodeDecodeError as e:
                errs.setdefault("partials", []).append([partial.hex(), None, str(e)[:60]])
            except Exception as e:
                errs.setdefault("partials", []).append([partial.hex(), None, type(e).__name__, str(e)[:60]])
            break
    lines.append("print(json.dumps({'enc': enc, 'dec': dec, 'errs': errs}))")

    # Run via subprocess to capture the dict cleanly.
    import subprocess
    full = "\n".join(["import codecs, json", f"info = codecs.lookup({key!r})",
                      f"print(info.name)"])
    # We already have lines that reference enc/dec/errs - build a single source.
    src = "\n".join([
        "import codecs, json",
        f"key = {key!r}",
        "enc = {}",
        "dec = {}",
        "SAMPLES = json.loads(r'''%s''')" % json.dumps(SAMPLES),
        "for samp in SAMPLES:",
        "    try:",
        "        b = samp.encode(key)",
        "    except UnicodeEncodeError:",
        "        continue",
        "    try:",
        "        enc[repr(samp)] = b.hex()",
        "    except Exception as e:",
        "        enc[repr(samp)] = None",
        "for samp in SAMPLES:",
        "    try:",
        "        b = samp.encode(key)",
        "    except UnicodeEncodeError:",
        "        continue",
        "    try:",
        "        t = b.decode(key)",
        "        dec[b.hex()] = [t, %d]" % 0,
        "    except UnicodeDecodeError:",
        "        dec[b.hex()] = None",
        "    except Exception as e:",
        "        dec[b.hex()] = ['ERR', type(e).__name__]",
        "print(json.dumps({'enc': enc, 'dec': dec}))",
    ])
    proc = subprocess.run([REF, "-S", "-c", src], capture_output=True, text=True, timeout=30)
    if proc.returncode != 0:
        entry["ERROR"] = proc.stderr[:500]
    else:
        lines_out = proc.stdout.splitlines()
        entry["canonical"] = key
        try:
            data = json.loads(lines_out[0])
            entry["enc"] = data["enc"]
            entry["dec"] = data["dec"]
        except (IndexError, json.JSONDecodeError):
            entry["ERROR"] = proc.stdout[:500]
    # error-handler block (separate child, may raise)
    src2 = "\n".join([
        "import codecs, json",
        f"key = {key!r}",
        "SAMPLES = json.loads(r'''%s''')" % json.dumps(SAMPLES),
        "errs = {}",
        "for samp in SAMPLES:",
        "    try:",
        "        samp.encode(key)",
        "        continue",
        "    except UnicodeEncodeError:",
        "        pass",
        "    except Exception as e:",
        "        continue",
        "    for eh in ('replace','ignore','backslashreplace','xmlcharrefreplace'):",
        "        try:",
        "            errs.setdefault(repr(samp), {})[eh] = samp.encode(key, eh).hex()",
        "        except Exception as e:",
        "            errs.setdefault(repr(samp), {})[eh] = 'ERR:' + type(e).__name__ + ':' + str(e)[:60]",
        "    try:",
        "        samp.encode(key)",
        "    except UnicodeEncodeError as e:",
        "        errs.setdefault(repr(samp), {})['strict_attrs'] = [e.object if isinstance(e.object, str) else e.object.hex(), e.start, e.end, e.reason, e.args[4] if len(e.args) > 4 else '']",
        "    except Exception as e:",
        "        pass",
        "print(json.dumps(errs))",
    ])
    proc2 = subprocess.run([REF, "-S", "-c", src2], capture_output=True, text=True, timeout=30)
    if proc2.returncode != 0:
        entry["ERROR2"] = proc2.stderr[:500]
    else:
        try:
            entry["enc_errors"] = json.loads(proc2.stdout.strip().splitlines()[-1])
        except (json.JSONDecodeError, IndexError):
            entry["ERROR2"] = proc2.stdout[:200] + proc2.stderr[:200]

    # decode-errors block
    src3 = "\n".join([
        "import codecs, json",
        f"key = {key!r}",
        "SAMPLES = json.loads(r'''%s''')" % json.dumps(SAMPLES),
        "errs = {}",
        "for samp in SAMPLES:",
        "    try:",
        "        b = samp.encode(key)",
        "    except UnicodeEncodeError:",
        "        continue",
        "    # truncations at every length",
        "    for cut in range(1, len(b)):",
        "        p = b[:cut]",
        "        try:",
        "            t = p.decode(key)",
        "            errs.setdefault('partial', []).append([p.hex(), t])",
        "        except UnicodeDecodeError as e:",
        "            errs.setdefault('partial', []).append([p.hex(), None, [e.object.hex(), e.start, e.end, e.reason]])",
        "        except Exception as e:",
        "            errs.setdefault('partial', []).append([p.hex(), None, type(e).__name__, str(e)[:60]])",
        "print(json.dumps(errs))",
    ])
    proc3 = subprocess.run([REF, "-S", "-c", src3], capture_output=True, text=True, timeout=60)
    if proc3.returncode != 0:
        entry["ERROR3"] = proc3.stderr[:500]
    else:
        try:
            entry["partial_decodes"] = json.loads(proc3.stdout.strip().splitlines()[-1])
        except (json.JSONDecodeError, IndexError):
            entry["ERROR3"] = proc3.stdout[:200] + proc3.stderr[:200]

    # crafted invalid bytes
    bad = CRAFTED_INVALID.get(key)
    if bad is not None:
        src4 = "\n".join([
            "import codecs, json",
            f"key = {key!r}",
            f"bad = bytes.fromhex({bad.hex()!r})",
            "out = {}",
            "try:",
            "    out['strict'] = bad.decode(key)",
            "except UnicodeDecodeError as e:",
            "    out['strict'] = None; out['strict_attrs'] = [e.object.hex(), e.start, e.end, e.reason, str(e)]",
            "except Exception as e:",
            "    out['strict'] = None; out['strict_attrs'] = [type(e).__name__, str(e)[:80]]",
            "try:",
            "    out['replace'] = bad.decode(key, 'replace')",
            "except Exception as e:",
            "    out['replace'] = 'ERR:' + type(e).__name__ + ':' + str(e)[:60]",
            "try:",
            "    out['ignore'] = bad.decode(key, 'ignore')",
            "except Exception as e:",
            "    out['ignore'] = 'ERR:' + type(e).__name__ + ':' + str(e)[:60]",
            "print(json.dumps(out))",
        ])
        proc4 = subprocess.run([REF, "-S", "-c", src4], capture_output=True, text=True, timeout=30)
        if proc4.returncode != 0:
            entry["ERROR4"] = proc4.stderr[:500]
        else:
            try:
                entry["bad_bytes"] = json.loads(proc4.stdout.strip().splitlines()[-1])
            except (json.JSONDecodeError, IndexError):
                entry["ERROR4"] = proc4.stdout[:200]

    out[key] = entry

os.makedirs("tests/corpus/codecs_cjk", exist_ok=True)
with open("tests/corpus/codecs_cjk/ground_truth_py314.json", "w") as f:
    json.dump(out, f, indent=1, ensure_ascii=True)
print("wrote tests/corpus/codecs_cjk/ground_truth_py314.json")
errs = {k: v for k, v in out.items() if "ERROR" in v}
print("errors:", list(errs.keys()))
