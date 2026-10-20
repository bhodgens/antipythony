//! Integration gate tests for the xtask CLI itself (`notices --check`
//! exit-code semantics), run against an isolated fixture workspace in the
//! system temp dir. The repo's real THIRD_PARTY_NOTICES is NEVER written by
//! these tests; at most they read it.
//!
//! How the isolation works: `workspace_root()` in src/main.rs walks up from
//! `CARGO_MANIFEST_DIR`, and `collect_deps()`'s first `cargo metadata` call
//! runs in the process CWD. Pointing both at the fixture workspace and
//! running the compiled xtask binary with `cwd = fixture` makes the whole
//! tool operate on the fixture copy — no repo mutation, no new deps.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Unique tempdir per test (pid + test name), created fresh, removed at end.
fn fixture_dir(name: &str) -> PathBuf {
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("xtask-gate-fixture-{name}-{pid}"));
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }
    fs::create_dir_all(dir.join("crates").join("sample")).expect("create fixture dir");
    dir
}

/// Minimal valid cargo workspace: one member crate, no external deps, so
/// `cargo metadata` resolves offline without touching any registry.
fn write_fixture(root: &Path) {
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample\"]\n",
    )
    .unwrap();
    fs::write(
        root.join("crates").join("sample").join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\nlicense = \"MIT\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates").join("sample").join("src")).unwrap();
    fs::write(
        root.join("crates")
            .join("sample")
            .join("src")
            .join("lib.rs"),
        "",
    )
    .unwrap();
}

/// Run the compiled xtask binary with CWD (and CARGO_MANIFEST_DIR) inside the
/// fixture workspace.
fn run_xtask(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .current_dir(root)
        .env("CARGO_MANIFEST_DIR", root.join("crates").join("sample"))
        .output()
        .expect("spawn xtask binary")
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_exits_zero_when_notices_in_sync() {
    let root = fixture_dir("sync");
    write_fixture(&root);
    let notices = root.join("THIRD_PARTY_NOTICES");

    let gen = run_xtask(&root, &["notices"]);
    assert!(
        gen.status.success(),
        "fixture `notices` generation failed: {}",
        String::from_utf8_lossy(&gen.stderr)
    );
    assert!(notices.exists(), "notices must be written into the fixture");

    let check = run_xtask(&root, &["notices", "--check"]);
    assert_eq!(
        check.status.code(),
        Some(0),
        "--check on a freshly generated fixture must exit 0: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    // Determinism at the CLI level: regenerate and require byte-identity.
    let first = fs::read(&notices).unwrap();
    let regen = run_xtask(&root, &["notices"]);
    assert!(regen.status.success());
    let second = fs::read(&notices).unwrap();
    assert_eq!(first, second, "CLI re-generation must be byte-identical");

    cleanup(&root);
}

#[test]
fn check_exits_one_when_notices_stale() {
    let root = fixture_dir("stale");
    write_fixture(&root);
    let notices = root.join("THIRD_PARTY_NOTICES");

    assert!(run_xtask(&root, &["notices"]).status.success());
    // Inject staleness in the fixture only — never the repo file.
    let mut stale = fs::read_to_string(&notices).unwrap();
    stale.push_str("\nSTALE INJECTED ENTRY (fixture drift)\n");
    fs::write(&notices, stale).unwrap();

    let check = run_xtask(&root, &["notices", "--check"]);
    assert_eq!(
        check.status.code(),
        Some(1),
        "--check on stale notices must exit 1 (NOTICES DRIFT)"
    );
    assert!(
        String::from_utf8_lossy(&check.stderr).contains("NOTICES DRIFT"),
        "drift message expected on stderr"
    );

    cleanup(&root);
}

#[test]
fn check_exits_two_when_notices_missing() {
    let root = fixture_dir("missing");
    write_fixture(&root);

    let check = run_xtask(&root, &["notices", "--check"]);
    assert_eq!(
        check.status.code(),
        Some(2),
        "--check with no THIRD_PARTY_NOTICES file must exit 2"
    );

    cleanup(&root);
}

/// Read-only assertion of the LIVE CI gate (ci.yml fast-lints runs exactly
/// this command). It only reads repo files; it must stay green or the gate
/// in CI is red too.
#[test]
fn repo_notices_gate_is_in_sync() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").exists())
        .expect("workspace root above crates/xtask")
        .to_path_buf();

    let check = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["notices", "--check"])
        .current_dir(&repo_root)
        .output()
        .expect("spawn xtask binary");
    assert_eq!(
        check.status.code(),
        Some(0),
        "repo THIRD_PARTY_NOTICES is stale: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}
