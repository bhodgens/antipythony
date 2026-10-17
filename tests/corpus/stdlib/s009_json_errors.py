# Cites: S-05-4 (row json), C-14/C-25 message pins, S-06-7
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import io
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
print(json.JSONDecodeError is not None,
      issubclass(json.JSONDecodeError, ValueError))
try:
    json.load(io.StringIO("{bad"))
except json.JSONDecodeError as e:
    print("load-stream:", e.msg, e.lineno, e.colno)
print(json.dumps({"emoji": "\U0001F600"}))
print(json.dumps({"emoji": "\U0001F600"}, ensure_ascii=False))
