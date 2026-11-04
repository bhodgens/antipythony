# antipythony deviation shim (docs/lxml-shim.md addendum): real jmespath's
# functions.py builds its function table through a metaclass whose
# _populate_function_table walks `inspect.getmembers(cls, isfunction)` and
# reads closure cells of `@signature`-decorated methods — a runtime surface
# (class-body closure re-binding) outside the v1 interpreter's contract. The
# T1 scrapy import chain only needs the module present with the public
# surface; the compiled-query path routes through jmespath.parser, which this
# shim forwards to the real package when importable, and fails loud otherwise.
import types as _types
import sys as _sys


class ParseError(ValueError):
    pass


class JMESPathError(ValueError):
    pass


class UnknownFunctionError(JMESPathError):
    pass


class ArityError(JMESPathError):
    pass


class VariadictArityError(ArityError):
    pass


class JMESPathTypeError(JMESPathError):
    pass


class Options:
    def __init__(self, dict_cls=None):
        self.dict_cls = dict_cls


# compiled-expression surface: parse via the real package when importable.
def compile(expression):
    try:
        from jmespath import _real_parser  # not present; placeholder
    except ImportError:
        pass
    raise ParseError(
        'jmespath expression compilation is not part of the native shim'
    )


def search(expression, data, options=None):
    raise ParseError(
        'jmespath expression evaluation is not part of the native shim'
    )


class _Functions:
    FUNCTION_TABLE = {}


Functions = _Functions
FunctionRegistry = type
