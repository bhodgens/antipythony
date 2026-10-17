# Cites: S-05-4 (row json), S-05-22, anchor #5, C-26
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import io
import json
names = ["dump", "dumps", "load", "loads", "JSONDecoder", "JSONEncoder",
         "JSONDecodeError", "detect_encoding", "decoder", "encoder",
         "scanner"]
print(all(hasattr(json, n) for n in names), "_json" not in dir(json))
print(json.dumps({"b": 1, "a": [1, 2.5, True, None]}, sort_keys=True))
print(json.dumps({"b": 1, "a": 2}))
print(json.loads('{"x": [1, 2.0]}'))
print(json.loads('[1, 2.5, "x"]'))
d = json.loads('{"k1": 1, "k2": 2, "k0": 0}')
print(list(d.keys()))
print(json.dumps([1, 2], separators=(",", ":")))
print(json.dumps({"a": {"b": [1, 2]}}, indent=2, sort_keys=True))
print(json.dumps({"a": 1}, indent=None, separators=None))
print(json.dumps("café"), json.dumps("café", ensure_ascii=False))
print(json.dumps([float("inf"), float("-inf"), float("nan")]))
print(json.loads('[1e2]', parse_float=str) == ['1e2'])
rt = {"s": "x", "i": 1, "f": [1, {"g": None}], "b": True}
print(json.loads(json.dumps(rt)) == rt)
buf = io.StringIO()
json.dump({"z": [1]}, buf)
print(buf.getvalue())
print(json.detect_encoding(b'{"a":1}'))
class SetEnc(json.JSONEncoder):
    def default(self, o):
        if isinstance(o, set):
            return sorted(o)
        return super().default(o)
print(json.dumps({1, 2, 3}, cls=SetEnc))
dec = json.JSONDecoder()
print(dec.decode('[1,2]'), dec.decode('{"k": [1,2]}'))
print(json.decoder.JSONDecodeError is json.JSONDecodeError)
print(json.dumps({"t": (1, 2)}), json.dumps({2.0: "float-key"}))
print(json.dumps({"esc": "a\"b\\c\td"}))
print(sorted(json.loads('{"z":1,"a":2,"m":3}', object_pairs_hook=list)))
