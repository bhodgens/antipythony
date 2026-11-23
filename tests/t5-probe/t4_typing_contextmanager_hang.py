"""T5 Wall-1 repro: `from typing import ContextManager` hangs on antipythony.

CPython 3.14.7: runs green, prints DONE.
antipythony: hangs inside typing.__getattr__ alias resolution (rc=124 under
any external deadline; zero output past PRE line).
"""
import sys

print("PRE", flush=True)
import typing

print("has ContextManager:", hasattr(typing, "ContextManager"), flush=True)
print("pre-import", flush=True)
from typing import ContextManager  # HANGS on antipythony

print("post-import", flush=True)
from typing import AsyncContextManager  # would also hang

print("DONE", flush=True)
