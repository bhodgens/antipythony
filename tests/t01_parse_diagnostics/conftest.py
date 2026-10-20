"""robustness-ci 05.02/01.02 flip policy for the T-01 seam suites.

ap-parse-check (leaf 05.02) landed, so the blanket xfail(strict=False)
markers flip: genuinely-passing cases get their marker REMOVED (real
green runs); the measured known-fail set below keeps its lenient xfail
with the existing reason. Known-fail measured 2026-10-20 via
`pytest --runxfail` (exact tails listed):

 - F-1 (verify surface, owner: 01.04 product work): ruff parse is
   syntax-only, so break_outside_loop / nonlocal_module /
   star_import_in_function ACCEPT at parse where CPython rejects at
   compile/symtable stage.
 - F-2 (contract PR material): ruff-vs-CPython position drift on
   unclosed_paren / unicode_unclosed_paren / py2_print / invalid_fstring
   position arms, and the two suites' mutual py2_print column disagree-
   ment; unicode column contract gap (G-2).

When F-1/F-2 are fixed upstream these arms XPASS and the XPASS ceiling
gate (leaf 01.01) forces this list to shrink.
"""

from __future__ import annotations

import pytest

# exact test-nodeid tails (after "::") that still fail vs the seam
KNOWN_FAIL = frozenset(
    {
        # tests/t01_parse_diagnostics
        "test_invalid_program_rejected[break_outside_loop]",
        "test_invalid_program_rejected[nonlocal_module]",
        "test_invalid_program_rejected[star_import_in_function]",
        "test_diagnostic_has_file_line_col[break_outside_loop]",
        "test_diagnostic_has_file_line_col[nonlocal_module]",
        "test_diagnostic_has_file_line_col[star_import_in_function]",
        "test_diagnostic_position_matches_reference[break_outside_loop]",
        "test_diagnostic_position_matches_reference[invalid_fstring]",
        "test_diagnostic_position_matches_reference[nonlocal_module]",
        "test_diagnostic_position_matches_reference[py2_print]",
        "test_diagnostic_position_matches_reference[star_import_in_function]",
        "test_diagnostic_position_matches_reference[unclosed_paren]",
        "test_diagnostic_position_matches_reference[unicode_unclosed_paren]",
        "test_py2_print_rejected_c03",
        "test_star_import_in_function_s04_10",
        "test_nonlocal_module_level_s02_3",
        "test_unicode_column_is_characters_g2",
        # tests/t01_parse
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
    tail = nodeid.split("::", 1)[-1]
    return tail in KNOWN_FAIL


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        if not (
            item.nodeid.startswith("tests/t01_parse_diagnostics/")
            or item.nodeid.startswith("tests/t01_parse/")
        ):
            continue
        idx = next(
            (i for i, m in enumerate(item.own_markers) if m.name == "xfail"),
            None,
        )
        if idx is None or _is_known_fail(item.nodeid):
            continue  # keep the existing lenient marker + reason
        # The seam landed and this case genuinely runs green: REMOVE the
        # marker so it reports as a real pass (a strict xfail on a passing
        # test is an XPASS failure - the wrong flip). Same shape as ledger
        # commit d79508d "flip stale xfail markers".
        del item.own_markers[idx]
