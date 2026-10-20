"""robustness-ci 05.02/01.02 flip policy for tests/t01_parse.

Mirror of tests/t01_parse_diagnostics/conftest.py (read that docstring
for the F-1/F-2 rationale). Exact known-fail tails below.
F-3 (fixture returned a bare Path with no .run()) was fixed directly in
test_parse_diagnostics.py by wrapping the bin in _BinRunner.
"""

from __future__ import annotations

import pytest

KNOWN_FAIL = frozenset(
    {
        "test_invalid_exact_position[break_outside_loop]",
        "test_invalid_exact_position[nonlocal_module_level]",
        "test_invalid_exact_position[star_import_in_function]",
        "test_invalid_exact_position[unclosed_paren]",
        "test_error_format_regex[break_outside_loop]",
        "test_error_format_regex[nonlocal_module_level]",
        "test_error_format_regex[star_import_in_function]",
        "test_unicode_column_characters",
    }
)


def _is_known_fail(nodeid: str) -> bool:
    return nodeid.split("::", 1)[-1] in KNOWN_FAIL


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        if not item.nodeid.startswith("tests/t01_parse/"):
            continue
        idx = next(
            (i for i, m in enumerate(item.own_markers) if m.name == "xfail"),
            None,
        )
        if idx is None or _is_known_fail(item.nodeid):
            continue
        del item.own_markers[idx]
