//! robustness-ci leaf 04.02 — proptest differential repr fuzz vs live CPython 3.14.
//!
//! Contract clauses exercised (cited per test):
//!   - docs/spec/01-values-and-objects.md S-01-41 (MUST): `repr(float)` byte-exact
//!     with CPython for every float — shortest round-trip digits, fixed notation
//!     for 1e-4 <= |x| < 1e16, exponential `Xe±NN` (>=2 exponent digits) outside,
//!     trailing `.0` on integral fixed values, specials `inf`/`-inf`/`nan`,
//!     `-0.0` keeps sign, `str(float) == repr(float)`.
//!   - docs/compat-contract.md C-15 (v1-locked): byte-exact float repr including
//!     the `1e+16` vs `1000000000000000.0` threshold and subnormal `5e-324`.
//!   - docs/compat-contract.md C-16: specials render exactly `inf` / `-inf` / `nan`.
//!   - docs/compat-contract.md C-17: round-trip `float(repr(v)) == v` and repr
//!     idempotence (checked impl-side in float_repr_roundtrips).
//!   - docs/spec/01-values-and-objects.md S-01-40 (MUST): `repr(int)` == plain
//!     decimal digits (no `+`, no leading zeros, `-` only when negative),
//!     `repr(-0) == repr(0) == 0`, `str(int) == repr(int)`.
//!   - docs/compat-contract.md C-14 digit limit: generated ints stay <= 800
//!     digits (under the default 4300 limit) so both sides render; over-limit
//!     behavior is pinned by unit tests in int.rs, not fuzzed here.
//!
//! Impl seam (cited per leaf brief option): DIRECT IN-PROCESS. The functions
//! under test are public API of ap-obj — `ap_obj::float_repr::py_float_repr`
//! (the hand-written formatter, the classic bug farm) and the engine dispatch
//! surfaces `ap_obj::ops::repr` / `ap_obj::ops::to_str` on `Value::Float` /
//! `Value::Int` (ops::repr's Float arm forwards to py_float_repr via
//! Value::fmt_repr, so both the formatter and the dispatch path are checked).
//! Reference bytes come from one batched python3.14 subprocess per chunk of
//! CHUNK=50 values (spawn cost amortized; values travel as bit patterns /
//! decimal strings, one line per output, so ordering is exact). The ap-obj
//! reference-discovery seam (`tests/common/mod.rs`, robustness-ci 00.02)
//! gates every differential test: SKIP without python3.14, hard-fail under
//! AP_REQUIRE_REFERENCE=1.
//!
//! NaN payload note: probed live on CPython 3.14.7
//! (plans/robustness-ci/04-fuzz/probe_0402.py): EVERY NaN bit pattern —
//! payload bits, quiet-bit 0, sign bit set — prints as exactly `nan`
//! (struct.unpack over 0x7ff8..01, 0x7ff4..01, 0xffff..ff, 0x7fff..ff,
//! 0x7ff0..01, 0x7ff8abcdef012345, 0xfff8..ff all -> 'nan'; float('-nan')
//! likewise). Random-bit NaNs are therefore a valid fuzz domain, not a
//! whitelist: if the impl prints anything but 'nan' for some pattern, that
//! is a FINDING and the suite goes red on it.
//!
//! Case count: AP_FUZZ_CASES env (default 200 = quick lane; release runs
//! higher, e.g. 1200). Each proptest case consumes CHUNK values per
//! reference subprocess.
//!
//! Routed findings: the default lane routes TWO real, byte-verified
//! divergences of float_repr.rs (see compare_float_bytes below) to
//! plans/robustness-ci/04-fuzz/PROGRESS.md + FUZZ_SEEDS_0402.txt instead of
//! failing — product code is owned by Plan 02, this leaf must not touch it.
//! AP_FUZZ_STRICT=1 disables routing (everything goes red): run that lane
//! after the Plan 02 fix to prove the bugs are gone. The pin test
//! `routed_findings_pins_currently_reproduce` flips red ON PURPOSE when the
//! fix lands, forcing router removal — the routing can never outlive the bug
//! it hides.

use ap_obj::float_repr::py_float_repr;
use ap_obj::int::Int;
use ap_obj::ops::{repr as ops_repr, to_str as ops_to_str};
use ap_obj::value::Value;

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::io::Write;
use std::process::{Command, Stdio};

mod common;

/// Values per reference subprocess (spawn amortization; leaf-brief "50").
const CHUNK: usize = 50;

/// Max generated decimal digits for ints — safely under the C-14/S-01-40
/// default 4300 str-digit limit so both sides always render.
const MAX_INT_DIGITS: usize = 800;

fn fuzz_cases() -> u32 {
    std::env::var("AP_FUZZ_CASES")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(200)
}

/// The pure comparator, isolated so a self-test can prove it detects
/// divergence (harness-can-fail guard; see comparator_rejects_synthetic_mismatch
/// plus the permanent AP_FUZZ_STRICT red-lane proof documented in PROGRESS).
fn check_pair(label: &str, impl_bytes: &str, ref_bytes: &str) -> Option<String> {
    if impl_bytes == ref_bytes {
        None
    } else {
        Some(format!(
            "{label}: impl={impl_bytes:?} reference={ref_bytes:?}"
        ))
    }
}

// -- routed findings (real divergences found by this leaf; Plan 02 owns
// float_repr.rs, so they route to PROGRESS instead of a silent whitelist) ---
//
// F0402-1 (layout): py_float_repr computes the decimal exponent from the
// integer-part LENGTH of Rust's `{v:?}` without stripping the `-` sign, so
// every NEGATIVE value with 1e15 <= |v| < 1e16 whose shortest digits are
// integral gets exp10 = len("-1234567890123456") - 1 = 16 and is re-laid
// into exponential form: impl `-<16 digits>e+16` where the reference prints
// fixed `-<16 digits>.0`. Root cause localized by read of
// float_repr.rs:55-67 (the `s.len()` branch counts the sign).
//
// F0402-2 (tie-break): when the exact binary value sits at the decimal
// midpoint between two adjacent shortest-length candidates, CPython's
// David Gay repr (mode "r") breaks the tie to the EVEN last digit; the impl
// (Rust `{v:?}` digit selection, half-up) emits the ODD-last-digit
// candidate. Bytes differ in exactly the final digit (odd vs even, delta 1);
// both candidates round-trip to the same f64 (that round-trip check is part
// of the signature — it cannot swallow a wrong-digit bug).
//
// Routing matches the EXACT structural signature of the two classes above,
// never "close". Any other mismatch hard-fails.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Finding {
    F0402_2,
}

fn strict_mode() -> bool {
    match std::env::var("AP_FUZZ_STRICT") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

/// F0402-2 signature: equal length, equal prefix, final digit odd (impl) vs
/// even (reference), delta 1, and BOTH byte strings round-trip to v's bits —
/// the midpoint tie-break, provably not a wrong-digit-count bug.
fn is_tie_signature(impl_s: &str, ref_s: &str, v: f64) -> bool {
    let ib = impl_s.as_bytes();
    let rb = ref_s.as_bytes();
    if ib.len() != rb.len() || ib.is_empty() || ib[..ib.len() - 1] != rb[..rb.len() - 1] {
        return false;
    }
    let (li, lr) = (ib[ib.len() - 1], rb[rb.len() - 1]);
    li.is_ascii_digit()
        && lr.is_ascii_digit()
        && li % 2 == 1
        && lr % 2 == 0
        && (li as i32 - lr as i32).abs() == 1
        && impl_s
            .parse::<f64>()
            .is_ok_and(|p| p.to_bits() == v.to_bits())
        && ref_s
            .parse::<f64>()
            .is_ok_and(|p| p.to_bits() == v.to_bits())
}

fn route_known_divergence(v: f64, impl_s: &str, ref_s: &str) -> Option<Finding> {
    if strict_mode() {
        return None;
    }
    if is_tie_signature(impl_s, ref_s, v) {
        return Some(Finding::F0402_2);
    }
    // F0402-1 (negative 1e15..1e16 layout window) was FIXED 2026-10-21
    // (float_repr.rs sign-stripping in the exp10 computation, wave-4
    // bughunt); its router arm was deleted per the pin-flip protocol.
    let _ = v;
    None
}

// F0402-1 shape check WITHOUT a reference string (the C-17 round-trip test
// has none). FIXED 2026-10-21 (wave-4 bughunt): the window no longer
// produces the layout bug, so the router arm and this shape check are
// deleted per the pin-flip protocol; the round-trip test now covers these
// values like any other.

static ROUTED_2: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn report_routed(test: &str) {
    let b = ROUTED_2.load(std::sync::atomic::Ordering::Relaxed);
    if b > 0 {
        eprintln!(
            "ROUTED FINDINGS ({test}): F0402-2 tie={b} \u{2014} see \
             plans/robustness-ci/04-fuzz/PROGRESS.md; AP_FUZZ_STRICT=1 runs these red"
        );
    }
}

/// Compare one (v, impl, ref) triple: hard-fail on any byte mismatch that is
/// not an EXACT match to a routed finding signature.
fn compare_float_bytes(label: &str, v: f64, impl_s: &str, ref_s: &str) -> Result<(), String> {
    if impl_s == ref_s {
        return Ok(());
    }
    match route_known_divergence(v, impl_s, ref_s) {
        Some(Finding::F0402_2) => {
            ROUTED_2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        None => Err(check_pair(label, impl_s, ref_s).unwrap_or_default()),
    }
}

/// One batched reference run: `script` reads whitespace-separated tokens from
/// stdin and prints exactly `tokens.len() * lines_per_token` lines.
/// Returns the lines (trailing empty line from the final \n is dropped).
fn ref_batch(
    program: &str,
    script: &str,
    tokens: &[String],
    lines_per_token: usize,
) -> Vec<String> {
    // 2026-11-06 harness review: single-tenancy + preflight + bounded wait —
    // a reference run must never hang the suite unboundedly.
    common::interpreter_gate();
    common::preflight_or_panic();
    let mut child = Command::new(program)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reference python3.14");
    // take() (not as_mut): the handle must DROP after the token loop so the
    // child sees EOF on stdin. Scripts read to EOF (read()/read().split()),
    // so a still-open pipe blocks them forever and every batch burned the
    // full reference deadline (2026-11-07: shrink-loop / gate-hold root cause).
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for t in tokens {
            writeln!(stdin, "{t}").expect("write token");
        }
    } // stdin dropped here -> EOF delivered
    let deadline = std::time::Instant::now() + common::reference_deadline();
    let timed_out = loop {
        match child.try_wait().expect("wait reference") {
            Some(_) => break false,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    assert!(
        !timed_out,
        "reference python3.14 exceeded the {}s deadline (AP_REF_TIMEOUT_SECS)",
        common::reference_deadline().as_secs()
    );
    let out = child.wait_with_output().expect("wait reference");
    assert!(
        out.status.success(),
        "reference python3.14 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\n')
        .map(|s| s.to_string())
        .collect();
    if lines.last().map_or(false, |l| l.is_empty()) {
        lines.pop();
    }
    assert_eq!(
        lines.len(),
        tokens.len() * lines_per_token,
        "reference protocol arity mismatch"
    );
    lines
}

/// Reference side for float bit patterns: struct.unpack('>d') of the u64 hex,
/// print repr then str (S-01-41 pins str == repr; we verify it on BOTH sides).
const FLOAT_SCRIPT: &str = "\
import sys, struct
for tok in sys.stdin.buffer.read().split():
    v = struct.unpack('>d', struct.pack('>Q', int(tok, 16)))[0]
    sys.stdout.write(repr(v) + chr(10))
    sys.stdout.write(str(v) + chr(10))
";

/// Reference side for ints: int(token, 10) (accepts optional +/- and leading
/// zeros exactly as the impl's parse_base10 does), repr then str.
const INT_SCRIPT: &str = "\
import sys
for tok in sys.stdin.buffer.read().split():
    n = int(tok, 10)
    sys.stdout.write(repr(n) + chr(10))
    sys.stdout.write(str(n) + chr(10))
";

fn float_strategy() -> impl Strategy<Value = u64> {
    // Two domains (C-15 Verify: "random bit patterns + boundary set"):
    // 1. pure bit-random f64 — NaN payloads, sign/extreme exponents,
    //    subnormals, +-inf, +-0.0, everything the exponent space covers.
    // 2. decimal-random doubles — realistic shortest-round-trip shapes, the
    //    domain the formatter's re-layout branch is the bug farm for.
    prop_oneof![
        any::<u64>(),
        (
            any::<bool>(),                  // sign
            1u64..1_000_000_000_000_000u64, // significand digits (1..15)
            0usize..16usize,                // fractional split point
            -330i32..330i32,                // decimal exponent
        )
            .prop_map(|(neg, sig, frac, exp)| {
                let s = sig.to_string();
                let frac = frac.min(s.len().saturating_sub(1));
                let (head, tail) = s.split_at(frac + 1);
                let text = if tail.is_empty() {
                    format!("{}{}e{}", if neg { "-" } else { "" }, head, exp)
                } else {
                    format!("{}{}.{}e{}", if neg { "-" } else { "" }, head, tail, exp)
                };
                // Parse with correct decimal -> binary64 rounding; overflow
                // (exp near +-330 with big mantissa) maps to +/-inf bits,
                // still a legal repr domain (C-16).
                let v: f64 = text.parse().unwrap_or(f64::INFINITY);
                v.to_bits()
            }),
    ]
}

fn int_token_strategy() -> impl Strategy<Value = String> {
    // Tokens as CPython int() accepts them: optional sign, optional leading
    // zeros (S-01-40: repr must strip them; repr(-0)==0), spanning the Smi
    // domain (any i64) and the Big domain (1..=800 random digits, kept under
    // the C-14 default 4300 str-digit limit so both sides always render).
    prop_oneof![
        any::<i64>().prop_map(|v| v.to_string()),
        (
            any::<bool>(),
            prop::collection::vec(any::<u8>(), 1..MAX_INT_DIGITS)
        )
            .prop_map(|(neg, bytes)| {
                let digits: String = bytes.iter().map(|b| (b'0' + (b % 10)) as char).collect();
                format!("{}{}", if neg { "-" } else { "" }, digits)
            }),
    ]
}

/// Deterministic boundary set from C-15 Verify + S-01-40/41 examples,
/// as float bit patterns (hex) — single batched reference run, zero proptest
/// nondeterminism. Doubles as the pinned regression table for the classic
/// threshold bugs (`1e+16`, `5e-324`, `-0.0`, NaN names).
fn float_boundary_bits() -> Vec<u64> {
    let vals: Vec<f64> = vec![
        0.0,
        -0.0,
        5e-324,                  // smallest subnormal (C-15 named byte string)
        2.2250738585072014e-308, // smallest normal
        1e-16,
        1e-7,
        3.5e-5,
        1e-4, // fixed/exponential threshold |x| >= 1e-4
        1.0 / 3.0,
        2.0 / 3.0,
        1.0,
        0.1,
        1e15,               // last fixed integral (positive)
        -1e15,              // F0402-1 minimal window member (negative)
        9999999999999998.0, // fixed just below 1e16
        1e16,               // first exponential (C-15 named)
        1e17,
        10000000000000001.0, // rounds to shortest digits 1e16 -> '1e+16'
        123456789012345.0,
        1.7976931348623157e308,             // max finite
        f64::from_bits(0x7ff0000000000000), // +inf
        f64::from_bits(0xfff0000000000000), // -inf
        f64::from_bits(0x7ff8000000000000), // quiet nan
        f64::from_bits(0x7fffffffffffffff), // max NaN payload
        f64::from_bits(0xfff8000000000000), // negative NaN
    ];
    vals.iter().map(|v| v.to_bits()).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]

    /// S-01-41 / C-15 / C-16: repr(float) + str(float) byte-exact for random
    /// bit patterns (NaN payloads, subnormals, inf, extremes, -0.0), against
    /// live CPython 3.14, checked through BOTH py_float_repr (the hand-written
    /// formatter) and ops::repr/ops::to_str (the engine dispatch surface).
    #[test]
    fn float_bit_repr_matches_cpython(chunk in prop::collection::vec(float_strategy(), CHUNK)) {
        let py = match common::reference_or_skip() { Some(p) => p, None => { return Ok(()); } };
        let tokens: Vec<String> = chunk.iter().map(|b| format!("{b:016x}")).collect();
        let lines = ref_batch(py, FLOAT_SCRIPT, &tokens, 2);
        for (i, bits) in chunk.iter().enumerate() {
            let v = f64::from_bits(*bits);
            let ref_repr = &lines[i * 2];
            let ref_str = &lines[i * 2 + 1];
            // formatter under test (the classic bug farm):
            let fmt = py_float_repr(v);
            compare_float_bytes(
                &format!("float repr {bits:016x} ({v:?})"),
                v,
                &fmt,
                ref_repr,
            )
            .map_err(TestCaseError::fail)?;
            // engine dispatch surface (Value::Float -> fmt_repr -> py_float_repr):
            let val = Value::Float(v);
            let eng_repr = ops_repr(&val).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            compare_float_bytes(
                &format!("ops::repr float {bits:016x}"),
                v,
                &eng_repr,
                ref_repr,
            )
            .map_err(TestCaseError::fail)?;
            // S-01-41: str(float) == repr(float) on BOTH sides.
            let eng_str = ops_to_str(&val).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            compare_float_bytes(
                &format!("ops::to_str float {bits:016x}"),
                v,
                &eng_str,
                ref_str,
            )
            .map_err(TestCaseError::fail)?;
            prop_assert_eq!(ref_repr, ref_str, "reference itself violated str==repr");
        }
        report_routed("float_bit_repr_matches_cpython");
    }

    /// C-17 (round-trip companion, impl-side only — no reference spawn): for
    /// every non-NaN bit pattern, `float(impl_repr(v))` must return the
    /// identical bits (shortest-round-trip promise) and repr must be
    /// idempotent through it. Guards digit selection independent of the
    /// reference. F0402-1 breaks round-trip too ("-<d>e+16" parses elsewhere)
    /// — routed by the SAME window signature, red under AP_FUZZ_STRICT=1.
    #[test]
    fn float_repr_roundtrips(bits in float_strategy()) {
        let v = f64::from_bits(bits);
        let r = py_float_repr(v);
        if v.is_nan() {
            // C-16: all NaN patterns render exactly 'nan' (payload
            // canonicalization probed on 3.14.7 — module header).
            prop_assert_eq!(&r, "nan");
        } else {
            let back: f64 = r
                .parse()
                .map_err(|e| TestCaseError::fail(format!("repr {r:?} does not parse: {e}")))?;
            prop_assert_eq!(
                back.to_bits(),
                v.to_bits(),
                "{}",
                format!("repr lost round-trip for {v:?}")
            );
            prop_assert_eq!(
                py_float_repr(back),
                r,
                "{}",
                format!("repr not idempotent for {v:?}")
            );
        }
    }

    /// S-01-40: repr(int)/str(int) byte-exact for decimal ints (i64 domain
    /// plus 1..=800 random digit runs with optional +/- and leading zeros).
    /// Hex/oct/bin formats are NOT part of this engine surface: ap-obj's
    /// repr is decimal-only (ops::repr; no format(x,'#x') implementation in
    /// the repr seam — leaf-brief scope rule: restrict to what the engine
    /// surface implements, here repr/str/dec). Digit count kept <= 800
    /// (< the C-14 default 4300 limit) so the limit path never trips.
    #[test]
    fn int_repr_matches_cpython(chunk in prop::collection::vec(int_token_strategy(), CHUNK)) {
        let py = match common::reference_or_skip() { Some(p) => p, None => { return Ok(()); } };
        let lines = ref_batch(py, INT_SCRIPT, &chunk, 2);
        for (i, tok) in chunk.iter().enumerate() {
            let int = Int::parse_base10(tok)
                .map_err(|_| TestCaseError::fail(format!("impl parse_base10 rejected {tok:?}")))?;
            let val = Value::Int(int);
            let ref_repr = &lines[i * 2];
            let ref_str = &lines[i * 2 + 1];
            let eng_repr = ops_repr(&val).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            if let Some(msg) = check_pair(&format!("int repr of {tok}"), &eng_repr, ref_repr) {
                return Err(TestCaseError::fail(msg));
            }
            let eng_str = ops_to_str(&val).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            if let Some(msg) = check_pair(&format!("int str of {tok}"), &eng_str, ref_str) {
                return Err(TestCaseError::fail(msg));
            }
        }
    }
}

/// Deterministic boundary sweep (C-15 "Verify" list + S-01-40/41 named
/// bytes): one batched reference run, no proptest. Also pins the classic
/// regression table. Routed findings are reported but do not fail this
/// (except under strict); anything else does.
#[test]
fn float_boundary_table_matches_reference() {
    let Some(py) = common::reference_or_skip() else {
        return;
    };
    let bits = float_boundary_bits();
    let tokens: Vec<String> = bits.iter().map(|b| format!("{b:016x}")).collect();
    let lines = ref_batch(py, FLOAT_SCRIPT, &tokens, 2);
    let mut fails = Vec::new();
    for (i, b) in bits.iter().enumerate() {
        let v = f64::from_bits(*b);
        let got = py_float_repr(v);
        if got != lines[i * 2] && route_known_divergence(v, &got, &lines[i * 2]).is_none() {
            fails.push(format!(
                "{b:016x} ({v:?}): impl={got:?} reference={:?}",
                lines[i * 2]
            ));
        }
    }
    assert!(
        fails.is_empty(),
        "float boundary divergences:\n{}",
        fails.join("\n")
    );
}

/// S-01-40 named boundaries: `repr(10**25)`, `repr(-0)`, Smi box boundary
/// (SMI_MAX/+1, SMI_MIN/-1 via i128 so no overflow), leading-zero and
/// plus-sign canonicalization — deterministic, batched reference.
#[test]
fn int_boundary_table_matches_reference() {
    let Some(py) = common::reference_or_skip() else {
        return;
    };
    let tokens: Vec<String> = vec![
        "0".into(),
        "-0".into(),
        "+0".into(),
        "000000012345".into(),
        "-000000012345".into(),
        (ap_obj::int::SMI_MAX as i128 + 1).to_string(),
        ap_obj::int::SMI_MAX.to_string(),
        (ap_obj::int::SMI_MIN as i128 - 1).to_string(),
        ap_obj::int::SMI_MIN.to_string(),
        "10000000000000000000000000".into(), // S-01-40 named: 10**25
        "-10000000000000000000000000".into(),
        format!("1{}", "0".repeat(400)), // Big domain, well under C-14 limit
    ];
    let lines = ref_batch(py, INT_SCRIPT, &tokens, 2);
    let mut fails = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        let int = Int::parse_base10(tok).expect("parse boundary token");
        let got = ops_repr(&Value::Int(int)).expect("boundary repr within digit limit");
        if got != lines[i * 2] {
            fails.push(format!("{tok}: impl={got:?} reference={:?}", lines[i * 2]));
        }
    }
    assert!(
        fails.is_empty(),
        "int boundary divergences:\n{}",
        fails.join("\n")
    );
}

/// Harness-can-fail self-test (permanent): proves `check_pair` — the exact
/// comparator every differential assertion routes through — REJECTS a
/// synthetic one-byte divergence and produces a finding-shaped message with
/// both byte strings. If this ever passes the wrong way, the gate is
/// decorative, not real.
#[test]
fn comparator_rejects_synthetic_mismatch() {
    let ok = check_pair("probe", "1e+16", "1e+16");
    assert!(ok.is_none(), "comparator rejected equal bytes: {ok:?}");
    let bad = check_pair("probe", "1e+15", "1e+16").expect("comparator accepted a mismatch");
    assert!(
        bad.contains("impl=") && bad.contains("reference="),
        "finding message lost both byte strings: {bad}"
    );
    // the router must NOT swallow a mismatch that lacks both signatures:
    assert_eq!(route_known_divergence(1e15, "1e+15", "1e+16"), None);
    // F0402-1's router arm was deleted when the bug was fixed (wave-4);
    // the same shape must now route as NOTHING (hard-fail if it recurs):
    assert_eq!(
        route_known_divergence(-1e15, "-1000000000000000e+16", "-1000000000000000.0"),
        None,
        "F0402-1 layout bug recurred and would be silently routed"
    );
    // the router DOES recognize the remaining routed shape (default lane;
    // strict-lane disabling is env-gated and proven by the one-off
    // AP_FUZZ_STRICT=1 gate run documented in PROGRESS — mutating env here
    // would race the parallel proptest threads):
    assert_eq!(
        route_known_divergence(
            -15614122725348.3125,
            "-15614122725348.313",
            "-15614122725348.312"
        ),
        Some(Finding::F0402_2),
        "router lost F0402-2 recognition"
    );
}

/// Provenance pins for the two routed findings (leaf 04.02, routed to Plan
/// 02 — see plans/robustness-ci/04-fuzz/PROGRESS.md + FUZZ_SEEDS_0402.txt).
/// Asserts the CURRENT (buggy) impl bytes against REFERENCE bytes live-probed
/// through the same batched seam. When Plan 02 fixes float_repr.rs these
/// flip red ON PURPOSE: delete the router, the pins, and the finding
/// entries then. They exist so the routing is provably against real,
/// byte-verified divergences — not an untested guess.
#[test]
fn routed_findings_pins_currently_reproduce() {
    let Some(py) = common::reference_or_skip() else {
        return;
    };
    let tokens = [
        format!("{:016x}", (-1e15f64).to_bits()), // F0402-1 minimal window (FIXED)
        format!("{:016x}", (-15614122725348.3125f64).to_bits()), // F0402-2 scan-shrunk
    ];
    let lines = ref_batch(py, FLOAT_SCRIPT, &tokens, 2);
    // Reference bytes as live-probed (sanity that the seam matches the
    // recorded finding; FUZZ_SEEDS_0402.txt carries the same pairs):
    assert_eq!(lines[0], "-1000000000000000.0");
    assert_eq!(lines[2], "-15614122725348.312");
    // F0402-1 FIXED (wave-4 2026-10-21): impl must now MATCH the reference
    // exactly for the former divergent window value.
    let got1 = py_float_repr(-1e15);
    assert_eq!(
        got1, lines[0],
        "F0402-1 regressed: impl {got1:?} vs reference {:?}",
        lines[0]
    );
    // Impl still diverges on the tie-break (F0402-2, unfixed - Gay's
    // half-to-even vs Rust half-up):
    let got2 = py_float_repr(-15614122725348.3125);
    assert_eq!(
        route_known_divergence(-15614122725348.3125, &got2, &lines[2]),
        Some(Finding::F0402_2),
        "impl no longer produces F0402-2 bytes: {got2:?} vs {:?} \u{2014} Plan 02 fixed \
         it: delete the router + this pin + the PROGRESS entry",
        lines[2]
    );
}
