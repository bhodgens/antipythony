# antipythany deviation shim (DEVIATIONS.toml): pyexpat is a C extension
# (docs/compat-contract.md C-07). xmlrpc.client needs ParserCreate for the
# T1 gate; a full XML parser is out of scope, so the shim provides the name
# surface with parse-time NotImplementedError.
import sys as _sys


class error(Exception):
    pass


class ExpatError(error):
    pass


class model:  # pyexpat.model surface (ElementInfo etc. consumed via import)
    class ElementInfo:
        def __init__(self, name=None, is_empty=None):
            self.name = name
            self.is_empty = is_empty


class errors:  # pyexpat.errors surface
    XML_ERROR_NO_MEMORY = "out of memory"
    XML_ERROR_SYNTAX = "syntax error"
    XML_ERROR_NO_ELEMENTS = "no element found"
    XML_ERROR_INVALID_TOKEN = "not well-formed (invalid token)"
    XML_ERROR_UNCLOSED_TOKEN = "unclosed token"
    XML_ERROR_PARTIAL_CHAR = "partial character"
    XML_ERROR_TAG_MISMATCH = "mismatched tag"
    XML_ERROR_DUPLICATE_ATTRIBUTE = "duplicate attribute"
    XML_ERROR_JUNK_AFTER_DOC_ELEMENT = "junk after document element"
    XML_ERROR_PARAM_ENTITY_REF = "illegal parameter entity reference"
    XML_ERROR_UNDEFINED_ENTITY = "undefined entity"
    XML_ERROR_RECURSIVE_ENTITY_REF = "recursive entity reference"
    XML_ERROR_ASYNC_ENTITY = "asynchronous entity"
    XML_ERROR_BAD_CHAR_REF = "bad character reference"
    XML_ERROR_BINARY_ENTITY_REF = "binary entity reference"
    XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF = "attribute external entity reference"
    XML_ERROR_MISPLACED_XML_PI = "misplaced xml declaration"
    XML_ERROR_UNKNOWN_ENCODING = "unknown encoding"
    XML_ERROR_INCORRECT_ENCODING = "incorrect encoding"
    XML_ERROR_UNCLOSED_CDATA_SECTION = "unclosed CDATA section"
    XML_ERROR_EXTERNAL_ENTITY_HANDLING = "error in external entity handling"
    XML_ERROR_NOT_STANDALONE = "document is not standalone"
    XML_ERROR_UNEXPECTED_STATE = "unexpected parser state"


def ParserCreate(encoding=None, namespace_separator=None, intern=None):
    raise NotImplementedError(
        "pyexpat requires a C extension; XML parsing is not "
        "supported in this runtime"
    )


XML_PARAM_ENTITY_PARSING_NEVER = 0
XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE = 1
XML_PARAM_ENTITY_PARSING_ALWAYS = 2

__all__ = [
    "error", "ExpatError", "model", "errors", "ParserCreate",
    "XML_PARAM_ENTITY_PARSING_NEVER",
    "XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE",
    "XML_PARAM_ENTITY_PARSING_ALWAYS",
]

_sys.modules["xml.parsers.expat.model"] = model
_sys.modules["xml.parsers.expat.errors"] = errors
