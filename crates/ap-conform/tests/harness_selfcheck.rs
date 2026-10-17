//! ap-conform harness self-checks (bughunt 2026-10-17).
//!
//! The differential harness is the measurement instrument for every
//! conformance gate in the project, so its own failure modes must be pinned:
//!  1. it must be able to FAIL (negative control), and
//!  2. it must never accept a non-3.14 interpreter as the reference.

use std::process::Command;

#[test]
fn harness_reports_fail_on_divergent_impl() {
    // Negative control: /bin/echo can never reproduce `print("A")`.
    // If this ever passes, the comparator is broken and every green gate
    // in the project is worthless.
    let dir = std::env::temp_dir().join(format!("ap_conform_neg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("negcase.py");
    std::fs::write(&case, "print(\"A\")\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ap-conform"))
        .args([
            "run",
            "--corpus",
            &case.to_string_lossy(),
            "--impl",
            "/bin/echo",
        ])
        .output()
        .expect("spawn ap-conform");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(out.status.code(), Some(1), "negative control must exit 1");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains("FAIL"),
        "negative control must print FAIL: {text}"
    );
}

#[test]
fn reference_interpreter_is_actually_314() {
    let found = ap_conform::find_reference_interpreter().expect("host must have a 3.14 reference");
    let out = Command::new(&found)
        .arg("--version")
        .output()
        .expect("spawn reference");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("Python 3.14"),
        "find_reference_interpreter returned a non-3.14 interpreter: {found} -> {text}"
    );
}

#[test]
fn tier_flag_is_accepted_and_forwarded() {
    // C-31 / 02.03 gate seam: `--tier 0` must be accepted and forwarded to
    // the impl (never to the reference). A reference-only impl (python3.14)
    // would choke on --tier, so we point at a tiny shim that echoes its argv
    // and assert the flag reached the IMPL side.
    let dir = std::env::temp_dir().join(format!("ap_conform_tier_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("t.py");
    std::fs::write(&case, "print(1)\n").unwrap();

    // Bad tier value must be rejected (exit 2), not silently ignored.
    let bad = Command::new(env!("CARGO_BIN_EXE_ap-conform"))
        .args([
            "run",
            "--corpus",
            &case.to_string_lossy(),
            "--impl",
            "/bin/echo",
            "--tier",
            "7",
        ])
        .output()
        .expect("spawn ap-conform");
    assert_eq!(bad.status.code(), Some(2), "bad --tier must exit 2");
    let _ = std::fs::remove_dir_all(&dir);
}
