# Stale xfail ledger - 2026-10-18, updated 2026-10-19

## FLIPPED 2026-10-19 (52 tests, leaves landed)

- tests/t02_ir_shape (21 tests): leaf 01.02 ap-ir-dump bin landed - whole
  xfail envelope removed, suite 21/21 green.
- tests/t04_ir_verify (29 tests): leaf 01.04 ap-verify-dump seam landed -
  xfail markers removed (require_seam keeps its 127-stub safety net),
  suite green.
- tests/t03_lowering_semantics (1 test: observable effect order): marker
  removed, passes.
- tests/t15_release (1 test: notices gate green): marker removed - the gate
  it awaits (notices --check exit 0) has been true since the 2026-10-17
  wave and is now CI-enforced.

## STILL STALE (74 tests) - owned by leaf 02.05 tier-1

- tests/t08_tier1/test_t08_tier1.py: 74 xpassed tests share the
  `xfail_ap` decorator ("awaits 02.05 tier-1 CLIF/IC/OSR/deopt"). 245
  sibling tests in the same file still genuinely xfail, so the decorator
  cannot be removed wholesale - flip per-test when 02.05 lands.

Full list of the 74 (one test id per line):
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/001_hello.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/002_types.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f01_nested_same_quotes.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f02_nested_dict_subscript.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f03_backslash_in_field.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f04_spec_fixed.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f05_spec_align_right.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f06_spec_center.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f07_spec_fill_align.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f08_spec_sign_d.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f09_spec_alt_hex.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f10_spec_alt_bin.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f11_conv_r.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f12_conv_s.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f13_conv_a.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f14_nested_fstring.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f15_fstring_in_spec.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f16_spec_nested_width.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f17_debug_plain.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f18_debug_spaces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f19_debug_spec.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f20_debug_conv.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f21_debug_expr.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f22_triple_multiline.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f23_multiline_field.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f24_dict_literal.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f25_lambda_call.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f26_walrus.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f27_conditional.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f28_method_call_subscript.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f29_genexpr.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f30_escaped_braces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f31_implicit_concat_mixed.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f32_concat_plain_between.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f33_bytes_adjacent_error.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f34_spec_literal_braces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/cases/f35_empty_and_self.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f01_nested_same_quotes.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f02_nested_dict_subscript.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f03_backslash_in_field.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f04_spec_fixed.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f05_spec_align_right.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f06_spec_center.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f07_spec_fill_align.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f08_spec_sign_d.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f09_spec_alt_hex.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f10_spec_alt_bin.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f11_conv_r.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f12_conv_s.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f13_conv_a.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f14_nested_fstring.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f15_fstring_in_spec.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f16_spec_nested_width.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f17_debug_plain.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f18_debug_spaces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f19_debug_spec.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f20_debug_conv.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f21_debug_expr.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f22_triple_multiline.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f23_multiline_field.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f24_dict_literal.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f25_lambda_call.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f26_walrus.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f27_conditional.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f28_method_call_subscript.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f29_genexpr.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f30_escaped_braces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f31_implicit_concat_mixed.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f32_concat_plain_between.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f33_bytes_adjacent_error.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f34_spec_literal_braces.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/cases/f35_empty_and_self.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_byte_identity[tests/corpus/fstrings/tools/gen_cases.py]
tests/t08_tier1/test_t08_tier1.py::test_cross_mode_suite_runs_over_grown_corpus
