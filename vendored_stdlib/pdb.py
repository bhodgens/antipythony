"""pdb stub (antipythany v0): pytest's debugging support imports pdb for
--pdb; under our runtime an interactive debugger is inert. Provide the
minimal names CPython's pdb module exposes that callers may touch."""

__all__ = ["set_trace", "Pdb", "post_mortem", "pm", "run", "runcall", "runeval", "main"]


class Pdb:  # pragma: no cover - inert
    def __init__(self, *args, **kwargs):
        pass

    def set_trace(self, *args, **kwargs):
        return None


def set_trace(*args, **kwargs):
    return None


def post_mortem(t=None):
    return None


def pm():
    return None


def run(statement, globals=None, locals=None):
    return None


def runcall(function, *args, **kwargs):
    return function(*args, **kwargs)


def runeval(expression, globals=None, locals=None):
    return eval(expression, globals, locals)


def main():
    return None
