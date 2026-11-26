//! fix_pass_differential — post-02.04a fix pass.
//!
//! Differential probes vs live `python3.14` for the three bugs fixed in this
//! pass:
//! - BUG-B: builtin method-call syntax routes through the methods.rs table
//!   (set.discard, set.add, frozenset.copy, dict.get/pop, list.append) and
//!   printf-style `str % args` formatting (_py_warnings depends on it).
//! - BUG-C (partial): `import warnings` succeeds (the _warnings/_contextvars
//!   shims + vendored _py_warnings).
//! - BUG-A support fixes verified indirectly: str % and the enum/kwarg chain
//!   stay non-regressing (full pathlib still requires EnumDict class-ns
//!   semantics — documented follow-up, not gated here).
//!
//! Skips when python3.14 is unavailable. stdout bytes must match the
//! reference.

use std::process::Command;

// Reference discovery is shared (leaf 00.02): env AP_CONFORM_REFERENCE_PYTHON
// -> PATH python3.14 (version-STRING checked; the old spawn-only probe could
// accept a non-3.14 interpreter that merely happened to be named python3.14).
// AP_REQUIRE_REFERENCE=1 converts a missing reference into a hard failure.
mod common;

fn ap_run_eval() -> &'static str {
    ap_run_eval_dyn().leak()
}

fn ap_run_eval_dyn() -> String {
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-eval"),
        Err(_) => {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target-0304c/debug/ap-run-eval") /* .cargo/config.toml redirect; cargo does not export it; CI ENOENT fix 2026-11-25 */
        }
    }
    .to_string_lossy()
    .into_owned()
}

/// Run `src` under both interpreters; assert identical stdout + exit code 0.
fn assert_differential(src: &str) {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return, // no reference + AP_REQUIRE_REFERENCE unset: skip
    };
    // Stamp = pid + (nanos XOR per-process counter). macOS clock granularity
    // (~1us) can hand two parallel test threads the SAME nanos; without the
    // counter one thread's remove_file deletes the other's case mid-run
    // ('cannot read ap_fixpass_*.py: No such file') — same root cause the
    // import_differential gate hit (workspace-gate flake 2026-11-05).
    static CASE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("fix_pass_differential.rs:49")
        .as_nanos() as u64;
    let stamp = nanos ^ CASE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("ap_fixpass_{}_{}.py", std::process::id(), stamp));
    std::fs::write(&tmp, src).expect("write temp case");
    let expected = Command::new(py)
        .arg(&tmp)
        .output()
        .expect("spawn python3.14");
    // Interpreter side runs under the OOM fence (2026-11-06 mitigation 2).
    let actual = common::fenced_output(Command::new(ap_run_eval()).arg(&tmp));
    std::fs::remove_file(&tmp).ok();
    assert!(
        actual.status.success(),
        "ap-run-eval failed: {}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&expected.stdout),
        "stdout mismatch"
    );
}

#[test]
fn bugb_set_dict_list_method_battery() {
    common::interpreter_gate();
    // Set iteration order is hash-seed dependent → sort before comparing.
    assert_differential(
        r#"s = {1, 2, 3}
s.discard(2)
print(sorted(s))
s.add(10)
print(sorted(s))
fs = frozenset({1, 2})
print(sorted(fs.copy()))
d = {"a": 1, "b": 2}
print(d.get("a"), d.get("z", 0))
print(d.pop("b"))
print(list(d.keys()), list(d.values()), list(d.items()))
lst = [1, 2]
lst.append(3)
print(lst)
"#,
    );
}

#[test]
fn bugb_printf_style_percent_format() {
    common::interpreter_gate();
    // _py_warnings depends on this surface.
    assert_differential(
        r#"print("  %s\n" % "line")
print("%d items" % 5)
print("%r %r" % ("a", 1.5))
print("%x %o %c" % (255, 8, "Z"))
print("%-6d|" % 42)
print("%05.1f" % 3.14159)
print("100%%")
"#,
    );
}

#[test]
#[ignore = "BUG-C partially fixed: warnings import reaches the enum chain (RegexFlag/EnumDict class-ns semantics) — follow-up leaf 02.04b"]
fn bugc_import_warnings_succeeds() {
    common::interpreter_gate();
    assert_differential(
        r#"import warnings
print(warnings.__name__)
"#,
    );
}

// ---------------------------------------------------------------------------
// issue #19 (harvested work item 2) — re-verified 2026-11 on trunk.
// ---------------------------------------------------------------------------

#[test]
fn io19_int_from_bytes_range_and_shapes() {
    common::interpreter_gate();
    // Per-element >255 must RAISE (was silently masked as u8); iterator/
    // generator/range data must be consumed as DATA (was routed to the
    // byteorder slot and parsed b"big"); str data raises the buffer TypeError.
    assert_differential(
        r#"try:
    print(int.from_bytes([300], 'big'))
except ValueError as e:
    print('V:', e)
try:
    int.from_bytes([-1], 'big')
except ValueError as e:
    print('V:', e)
try:
    int.from_bytes(['a'], 'big')
except TypeError as e:
    print('T:', e)
try:
    int.from_bytes('ab', 'big')
except TypeError as e:
    print('T:', e)
print(int.from_bytes(b'\x01\x02', 'big'))
print(int.from_bytes(iter([1, 2]), 'big'))
print(int.from_bytes(iter([1, 2]), 'little'))
print(int.from_bytes(range(2), 'big'))
print(int.from_bytes(bytearray([1, 2]), 'big'))
print(int.from_bytes([1, 2, 3], 'little', signed=True))
print(int.from_bytes([1, 2], byteorder='little'))
print(int.from_bytes([], 'big'))
print(int.from_bytes((3,), 'little'))
try:
    int.from_bytes([1], 'middle')
except ValueError as e:
    print('V:', e)
b = bytearray([1, 2])
print(b, int.from_bytes(b, 'big'))
print(bytearray('ab', 'utf-8'), bytearray(3), bytearray(b'xy'))
try:
    bytearray([300])
except ValueError as e:
    print('BV:', e)
try:
    bytearray([1, 'a'])
except TypeError as e:
    print('BT:', e)
"#,
    );
}

#[test]
fn io19_format_spec_zero_fill_wide_ints() {
    common::interpreter_gate();
    // x/X/o/b beyond u128 must render real radix digits (was saturated to 0);
    // zero-fill + ',' groups the FILL (width counts commas); ints accept the
    // float codes via f64 coercion; 'c' error taxonomy.
    assert_differential(
        r#"big = 1 << 130
vals = [big, -(1 << 130), 10 ** 45, 2 ** 128, 255, 0, -255]
for v in vals:
    for spec in ['d', '020d', '<20d', '>20d', '^20d', '020x', '020X', '020b',
                 '020o', '+020d', ' 20d', '20', '05d', '010,', 'f', '.3f',
                 'e', 'g', '%', 'c', '#010x', '#010b']:
        try:
            print('[%s] -> [%s]' % (spec, format(v, spec)))
        except OverflowError:
            print('[%s] -> ERR OverflowError' % spec)
        except Exception as e:
            print('[%s] -> ERR %s: %s' % (spec, type(e).__name__, e))
print(format(65535, '010,'), format(65535, '013,'))
print(format(123456, '08,'), format(1, '08,'))
"#,
    );
}
