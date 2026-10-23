"""Stub of the _posixsubprocess C module (subprocess.py import-surface only).

The vendored subprocess.py imports fork_exec lazily inside `_use_posix_spawn`
detection; the v1 runtime never spawns child processes through the Python
subprocess module (the corpus gates don't exercise Popen), so the stub
fail-louds on use.
"""


def fork_exec(*args, **kwargs):
    raise NotImplementedError(
        "_posixsubprocess.fork_exec is not part of the v1 runtime surface"
    )
