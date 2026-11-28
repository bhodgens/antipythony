//! fp-rollout 03 — per-function tier dispatch seam: THREE-WAY differential.
//!
//! Proves the mixed composition `outer(bytecode) -> mid(tier-1/adaptive) ->
//! inner(bytecode)` and its inverse run in ONE process, and that both layers
//! execute (not a silent tree-walk fallback):
//!
//! 1. stdout/stderr/rc byte-identical vs CPython 3.14
//!    (`/opt/homebrew/bin/python3.14`, resolved via the shared ladder in
//!    `tests/common/mod.rs`, overridable with AP_CONFORM_REFERENCE_PYTHON);
//! 2. byte-identical vs the tree0 control (`ap-run-eval --tier 0`);
//! 3. `[TIER-SEAM-STATS] bytecode_execs>0 adaptive_execs>0` and
//!    `[TIERUP-STATS] tier1>0` — the tier-1 fast path actually installed;
//! 4. a 4-thread Barrier variant with sorted output so scheduling noise
//!    cannot make the comparison flaky.
//!
//! Harness discipline mirrors `tier_mode_differential.rs`: repo interpreter
//! gate + memory-pressure preflight + bounded per-probe deadline, diagnostic
//! env scrub (an inherited `APX_TIER_STATS` must not leak into the byte
//! comparison). Probes are embedded here (the dedicated new test file is
//! the only test surface this leaf owns) and written to a per-test temp dir.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

mod common;
use common::diagnostic_env;

fn interpreter_gate() {
    common::interpreter_gate();
}

#[cfg(target_os = "macos")]
fn memory_pressure_ok() -> bool {
    // The repo's canonical harness uses sysctlbyname here; this leaf adds
    // no new unchecked blocks (issue #15 ratchet). A bounded preflight via
    // vm_stat's page-free count is enough to refuse an obviously wedged
    // host; on any read failure the gate is left open.
    let out = match std::process::Command::new("vm_stat").output() {
        Ok(o) => o,
        Err(_) => return true,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let page_size = 16_384u64; // Apple silicon
    let free_pages = text
        .lines()
        .find_map(|l| l.strip_prefix("Pages free:"))
        .and_then(|v| {
            v.trim()
                .trim_end_matches('.')
                .replace(',', "")
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);
    free_pages.saturating_mul(page_size) > 2 * 1024 * 1024 * 1024 || free_pages == 0
}

#[cfg(not(target_os = "macos"))]
fn memory_pressure_ok() -> bool {
    if let Ok(avail) = std::fs::read_to_string("/proc/meminfo") {
        for line in avail.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                let kb: u64 = rest
                    .trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse()
                    .unwrap_or(0);
                return kb > 4 * 1024 * 1024;
            }
        }
    }
    true
}

fn preflight_or_panic() {
    assert!(
        memory_pressure_ok(),
        "memory-pressure preflight: machine under critical memory pressure; \
         tier-seam differential REFUSED to start"
    );
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write an embedded probe to a unique temp file (fenced subprocess needs
/// a path, not stdin — CPython must see the same source bytes).
fn probe_file(name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fp-tier-seam-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create tier-seam probe dir");
    let path = dir.join(name);
    std::fs::write(&path, src).expect("write tier-seam probe");
    path
}

/// `outer(bytecode) -> mid(tier-1/adaptive) -> inner(bytecode)`.
/// `mid` uses the bench_a1 int-while shape so the adaptive arm actually
/// installs a tier-1 fast path (`[TIERUP-STATS] tier1>0`), not just the
/// baseline JIT. `inner` is bytecode-clean, `outer` combines both.
const CHAIN_SRC: &str = r#"
def inner(n):
    return n + 1

def mid(n):
    acc = 0
    i = 0
    while i < n:
        acc = acc + i
        i = i + 1
    return acc

def outer(n):
    return mid(n) + inner(2)

for _ in range(30):
    outer(2000)
print(outer(2000))
"#;

/// 4-thread variant: Barrier forces all workers to enter together, sorted
/// output removes scheduling noise. Each worker runs the mixed chain.
const THREAD_SRC: &str = r#"
import threading

def inner(n):
    return n + 1

def mid(n):
    acc = 0
    i = 0
    while i < n:
        acc = acc + i
        i = i + 1
    return acc

def outer(n):
    return mid(n) + inner(2)

# Warm the tier-1 fast path BEFORE the thread phase (tier-up needs ~30
# typed calls); output is fixed so it cannot add scheduling noise.
_warm = 0
for _ in range(30):
    _warm = _warm + outer(2000)
assert _warm == 30 * 1999003

bar = threading.Barrier(4)
out = []
lock = threading.Lock()

def worker(k):
    for _ in range(3):
        bar.wait()
        v = outer(200)
        with lock:
            out.append((k, v))
    bar.wait()

ts = [threading.Thread(target=worker, args=(k,)) for k in range(4)]
for t in ts:
    t.start()
for t in ts:
    t.join()
print(sorted(out))
"#;

fn run_timeout_secs() -> u64 {
    std::env::var("AP_TIER_SEAM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240)
}

/// One fenced subprocess. Returns (stdout, stderr, rc).
fn run_fenced(mut cmd: Command) -> (Vec<u8>, Vec<u8>, i32) {
    preflight_or_panic();
    interpreter_gate();
    let start = Instant::now();
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tier-seam probe");
    let deadline = Duration::from_secs(run_timeout_secs());
    loop {
        match child.try_wait().expect("wait tier-seam probe") {
            Some(status) => {
                let out = child.wait_with_output().expect("collect tier-seam output");
                return (out.stdout, out.stderr, status.code().unwrap_or(-1));
            }
            None => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("tier-seam probe exceeded {}s deadline", run_timeout_secs());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn cpython_cmd(probe: &Path) -> Command {
    let py = common::reference_or_skip()
        .expect("CPython 3.14 reference required for tier-seam differential");
    let mut cmd = diagnostic_env::clean_command(py);
    cmd.arg(probe);
    cmd
}

fn ap_cmd(tier: &str, spec: Option<&str>, probe: &Path, stats: bool) -> Command {
    // ap-run-probe is this crate's own bin (CARGO_BIN_EXE is available);
    // it reads APX_TIER / APX_TIER_SEAM_OVERRIDE from the env inside the
    // interpreter thread, exactly like the existing differential suites.
    let bin = env!("CARGO_BIN_EXE_ap-run-probe");
    let mut cmd = diagnostic_env::clean_command(bin);
    cmd.env("APX_TIER", tier);
    if let Some(spec) = spec {
        cmd.env("APX_TIER_SEAM_OVERRIDE", spec);
    }
    if stats {
        cmd.env("APX_TIER_STATS", "1");
        cmd.env("APX_TIER_SEAM_STATS", "1");
    }
    cmd.arg(probe);
    cmd
}

/// Strip only the opt-in diagnostic lines before a byte comparison
/// (stats runs add them to stderr; the program's own stderr must match).
fn program_stderr(stderr: &[u8]) -> String {
    let stripped = diagnostic_env::strip_diagnostic_lines(&String::from_utf8_lossy(stderr));
    // fp-rollout 03's own opt-in lines (the shared denylist predates them;
    // this leaf cannot edit the shared harness, so filter them here).
    stripped
        .lines()
        .filter(|line| !line.starts_with("[TIER-SEAM]") && !line.starts_with("[TIER-SEAM-STATS]"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_u64(stderr: &[u8], prefix: &str, key: &str) -> u64 {
    let text = String::from_utf8_lossy(stderr);
    for line in text.lines() {
        if line.starts_with(prefix) {
            for tok in line.split_whitespace() {
                if let Some(v) = tok.strip_prefix(key) {
                    if let Ok(n) = v.parse() {
                        return n;
                    }
                }
            }
        }
    }
    0
}

fn assert_three_way(label: &str, probe: &Path, spec: &str) {
    // Three legs: CPython, tree0 control, mixed override run (with stats).
    let (py_out, py_err, py_rc) = run_fenced(cpython_cmd(probe));
    let (tree_out, tree_err, tree_rc) = run_fenced(ap_cmd("0", None, probe, false));
    let (mix_out, mix_err, mix_rc) = run_fenced(ap_cmd("bytecode", Some(spec), probe, true));

    eprintln!("=== {label} ===");
    eprintln!(
        "rc: cpython={py_rc} tree0={tree_rc} mixed={mix_rc}; \
         stdout bytes: cpython={} tree0={} mixed={}",
        py_out.len(),
        tree_out.len(),
        mix_out.len()
    );
    eprintln!("mixed stats stderr: {}", String::from_utf8_lossy(&mix_err));

    assert_eq!(
        py_rc, tree_rc,
        "{label}: tree0 rc diverges from CPython 3.14"
    );
    assert_eq!(
        py_rc, mix_rc,
        "{label}: mixed rc diverges from CPython 3.14"
    );
    assert_eq!(
        py_out, tree_out,
        "{label}: tree0 stdout diverges from CPython"
    );
    assert_eq!(
        py_out, mix_out,
        "{label}: mixed stdout diverges from CPython"
    );
    assert_eq!(
        program_stderr(&py_err),
        program_stderr(&tree_err),
        "{label}: tree0 stderr diverges from CPython"
    );
    assert_eq!(
        program_stderr(&py_err),
        program_stderr(&mix_err),
        "{label}: mixed stderr diverges from CPython"
    );

    // Exercise proof: BOTH tiers ran, and the adaptive tier installed an
    // actual tier-1 fast path (not just the baseline JIT).
    let bc = parse_u64(&mix_err, "[TIER-SEAM-STATS]", "bytecode_execs=");
    let jit = parse_u64(&mix_err, "[TIER-SEAM-STATS]", "adaptive_execs=");
    let tree = parse_u64(&mix_err, "[TIER-SEAM-STATS]", "tree_execs=");
    let tier1 = parse_u64(&mix_err, "[TIERUP-STATS]", "tier1=");
    let bc_layer = parse_u64(&mix_err, "[BC-STATS]", "execs=");
    let jit_layer = parse_u64(&mix_err, "[TIER-STATS]", "execs=");
    eprintln!(
        "{label}: seam bc={bc} adaptive={jit} tree={tree}; \
         tier1={tier1}; layer BC-STATS execs={bc_layer} TIER-STATS execs={jit_layer}"
    );
    assert!(bc > 0, "{label}: bytecode tier never executed (execs=0)");
    assert!(
        jit > 0,
        "{label}: adaptive/tier-1 tier never executed (execs=0)"
    );
    assert_eq!(
        tree, 0,
        "{label}: unexpected tree-walk fallback in mixed chain"
    );
    assert!(
        tier1 > 0,
        "{label}: tier-1 fast path never installed (tier1=0)"
    );
    assert!(
        bc_layer > 0,
        "{label}: BC-STATS execs=0 — vacuous byte compare"
    );
    assert!(
        jit_layer > 0,
        "{label}: TIER-STATS execs=0 — vacuous byte compare"
    );
}

#[test]
fn test_mixed_chain_forward_and_inverse_three_way() {
    let probe = probe_file("chain.py", CHAIN_SRC);
    assert_three_way(
        "bytecode -> tier1 -> bytecode",
        &probe,
        "bytecode:outer,adaptive:mid,bytecode:inner",
    );
    assert_three_way(
        "tier1 -> bytecode -> tier1 (inverse)",
        &probe,
        "adaptive:outer,bytecode:mid,adaptive:inner",
    );
}

#[test]
fn test_mixed_chain_four_threads_sorted_three_way() {
    let probe = probe_file("threads.py", THREAD_SRC);
    assert_three_way(
        "4-thread bytecode -> tier1 -> bytecode (Barrier, sorted)",
        &probe,
        "bytecode:outer,adaptive:mid,bytecode:inner",
    );
}

#[test]
fn test_bad_override_is_loud_not_silent() {
    let probe = probe_file("bad.py", CHAIN_SRC);
    let (out, err, rc) = run_fenced(ap_cmd(
        "bytecode",
        Some("bytecode:no_such_function"),
        &probe,
        false,
    ));
    eprintln!(
        "bad-spec rc={rc} stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    assert_ne!(rc, 0, "a malformed override spec must fail loudly");
    assert!(
        String::from_utf8_lossy(&err).contains("APX_TIER_SEAM_OVERRIDE rejected"),
        "bad spec must be named in stderr"
    );
}
