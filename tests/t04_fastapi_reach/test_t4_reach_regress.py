"""T4 reachability regression: surfaces landed for the fastapi hello-world path.

Pins (differential vs live CPython 3.14 where noted):
  * type.mro() — bound method returning a LIST (email/_policybase walks it).
  * PEP 634 class patterns — `case ast.If():` shape (anyio/_lazyimport AST walk).
  * module __getattr__ (PEP 562) may re-enter the module namespace
    (typing.ContextManager builds `bool | None` inside the hook) — must not
    self-deadlock the runtime.
  * set/frozenset do NOT answer __wrapped__ (inspect.unwrap must not descend
    into a frozenset stored in a class dict).
  * str classification surface readable OFF the type (Predicate(str.islower)).
"""
import subprocess
import shutil
import os.path as _osp
import pytest


def _bin():
    # The worktree layout: this file lives at <wt>/tests/t04_fastapi_reach/.
    wt = _osp.dirname(_osp.dirname(_osp.dirname(_osp.abspath(__file__))))
    for cand in (
        _osp.join(wt, "target-t4", "debug", "antipythony"),
        _osp.join(wt, "target", "debug", "antipythony"),
        shutil.which("antipythony"),
    ):
        if cand and _osp.isfile(cand):
            return cand
    pytest.skip("antipythony binary not found (build -p ap-cli)")


def _run(src: str, env_extra: dict | None = None, timeout: int = 120) -> subprocess.CompletedProcess:
    import tempfile, os
    d = tempfile.mkdtemp(prefix="t4reg_")
    p = os.path.join(d, "probe.py")
    with open(p, "w") as f:
        f.write(src)
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    return subprocess.run(
        [_bin(), "run", p], capture_output=True, text=True, env=env, timeout=timeout
    )


def _cpython(src: str, timeout: int = 60):
    import tempfile, os
    d = tempfile.mkdtemp(prefix="t4cpy_")
    p = os.path.join(d, "probe.py")
    with open(p, "w") as f:
        f.write(src)
    return subprocess.run(
        ["/opt/homebrew/bin/python3.14", p], capture_output=True, text=True, timeout=timeout
    )


def test_type_mro_returns_list_differential():
    src = (
        "class A: pass\n"
        "class B(A): pass\n"
        "print(type(B.mro()).__name__)\n"
        "print([c.__name__ for c in B.mro()])\n"
    )
    ours = _run(src)
    cpy = _cpython(src)
    assert ours.returncode == 0, ours.stderr[-400:]
    assert ours.stdout == cpy.stdout


def test_type_mro_is_callable_attr_on_interned_type():
    ours = _run("print(hasattr(str, 'islower'))\nprint(str.islower('AB'.lower()))\n")
    assert ours.returncode == 0, ours.stderr[-400:]
    assert ours.stdout == "True\nTrue\n"


def test_class_pattern_matches_builtin_ast_node():
    # anyio/_lazyimport walks `case ast.If():` — the engine must not raise
    # SystemError("class patterns need 02.03 classes").
    src = (
        "import ast\n"
        "tree = ast.parse('if x:\\n    pass')\n"
        "for node in tree.body:\n"
        "    match node:\n"
        "        case ast.If():\n"
        "            print('IF')\n"
        "        case _:\n"
        "            print('OTHER')\n"
    )
    cpy = _cpython(src)
    ours = _run(src)
    assert ours.returncode == 0, ours.stderr[-400:]
    assert ours.stdout == cpy.stdout == "IF\n"


def test_class_pattern_non_match_falls_through():
    src = (
        "import ast\n"
        "node = ast.parse('x = 1').body[0]\n"
        "for n in [node]:\n"
        "    match n:\n"
        "        case ast.If():\n"
        "            print('IF')\n"
        "        case _:\n"
        "            print('OTHER')\n"
    )
    cpy = _cpython(src)
    ours = _run(src)
    assert ours.returncode == 0, ours.stderr[-400:]
    assert ours.stdout == cpy.stdout == "OTHER\n"


def test_module_getattr_reentry_no_deadlock():
    # typing.ContextManager's PEP 562 hook builds `bool | None` (make_union
    # re-locks the typing module namespace). Regression: pre-fix this hung
    # forever (mutex held across the Python call, then dropped the wrong name).
    src = (
        "import typing\n"
        "v = typing.ContextManager\n"
        "print('OK', v.__name__ if hasattr(v, '__name__') else v)\n"
    )
    ours = _run(src, timeout=60)
    assert ours.returncode == 0, "HANG or error: " + ours.stderr[-400:]
    assert "OK" in ours.stdout


def test_frozenset_has_no_wrapped_attr():
    # inspect.unwrap descends __wrapped__; the set/frozenset virtual-dunder
    # fallback fabricated one and dataclasses._add_slots died on it.
    src = (
        "fs = frozenset({1})\n"
        "print(hasattr(fs, '__wrapped__'))\n"
        "s = set()\n"
        "print(hasattr(s, '__wrapped__'))\n"
    )
    cpy = _cpython(src)
    ours = _run(src)
    assert ours.returncode == 0, ours.stderr[-400:]
    assert ours.stdout == cpy.stdout == "False\nFalse\n"


def test_dataclass_slots_with_protocol_base():
    src = (
        "from typing import Protocol, runtime_checkable, Any\n"
        "import dataclasses\n"
        "@runtime_checkable\n"
        "class P(Protocol):\n"
        "    def __gt__(self, __other: Any) -> bool: ...\n"
        "@dataclasses.dataclass(frozen=True, slots=True)\n"
        "class Gt(P):\n"
        "    gt: Any\n"
        "print('DC-OK')\n"
    )
    ours = _run(src, timeout=120)
    assert ours.returncode == 0, ours.stderr[-400:]
    assert "DC-OK" in ours.stdout
