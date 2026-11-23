# Plan 05 T3 reachability driver — hermes-agent chat-turn-shaped path.
# Stage record (2026-11-23, see plans/antipythony/05-ecosystem-t2/NEXT-T3.md):
#   S0 GREEN  import pydantic_core (ce path, APX_EXT_DIR + deps/libpydantic_core.so)
#   S1 GREEN  import pydantic (pure-py face, staged manually into cache site-packages)
#   S2 BLOCKED from pydantic import BaseModel — re-entrant lazy-import cascade
#             (pydantic.main -> aliases -> collections.abc -> typing ->
#              pydantic.__getattr__ re-entry; F-STRUCT-1 in NEXT-T3.md)
#   S3 BLOCKED import httpx — vendored urllib.request lacks parse_http_list
#             (httpx/_auth.py:9; F-VENDORED-1)
#   S5 BLOCKED rich.panel — vendored colorsys missing (F-VENDORED-2; staged
#             beside this driver) then io.UnsupportedOperation: fileno in
#             rich terminal measure (F-INTERP-1)
# Kept as the standing repro driver for the next T3 leaf.
import sys

print("STAGE S0 import pydantic_core")
import pydantic_core as pc
print("S0 OK", type(pc.Model).__name__)

print("STAGE S1 import pydantic")
import pydantic
print("S1 OK", pydantic.VERSION)

print("STAGE S2 from pydantic import BaseModel")
from pydantic import BaseModel


class ChatMessage(BaseModel):
    role: str
    content: str
    turn: int


print("STAGE S2-instantiate")
msg = ChatMessage(role="user", content="hello", turn=1)
print("S2 OK", msg.role, msg.content, msg.turn)

print("STAGE S3 import httpx")
import httpx
client = httpx.Client(base_url="http://127.0.0.1:1")
print("S3 OK", type(client).__name__)

print("STAGE S4 build request (offline)")
req = client.build_request("POST", "/v1/chat", json={"role": "user", "content": "hi"})
print("S4 OK", req.method, req.url)
body = req.read()
print("S4B OK", len(body), type(body).__name__)

print("STAGE S5 import rich + render")
import io
from rich.console import Console
from rich.panel import Panel
buf = io.StringIO()
con = Console(file=buf, width=40, force_terminal=False, no_color=True)
con.print(Panel("hello, antipythony", title="T3"))
out = buf.getvalue()
print("S5 OK", len(out))
print("DONE", len(out) > 0)
