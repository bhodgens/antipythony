use std::time::{Duration, Instant};

// 2026-11-06 harness review: single-tenancy + memory-pressure preflight
// for this suite (same lock file / metrics as the other suites' common
// helpers — this target has no `mod common`, so the seam is local).
// Robustness leaf 00-ci-core/LEAF-gate-watchdog: bounded wait + holder
// liveness + stale takeover live in gate_watchdog.rs (include!d verbatim;
// canonical copy: crates/ap-obj/tests/common/gate_watchdog.rs).
include!("gate_watchdog.rs");

fn interpreter_gate() {
    gate_watchdog_acquire();
}

#[cfg(target_os = "macos")]
fn memory_pressure_ok() -> bool {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr() as *const libc::c_char,
            (&mut level as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return true; // metric unreadable: gate open
    }
    level <= 2
}

#[cfg(not(target_os = "macos"))]
fn memory_pressure_ok() -> bool {
    if let Ok(avail) = std::fs::read_to_string("/proc/meminfo") {
        for line in avail.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                if let Ok(kb) = rest.trim().trim_end_matches(" kB").trim().parse::<u64>() {
                    return kb > 4 * 1024 * 1024;
                }
            }
        }
    }
    true
}

fn preflight_or_panic() {
    assert!(
        memory_pressure_ok(),
        "memory-pressure preflight: machine under critical memory pressure; \
         ast-getattr fence probe REFUSED to start — free the machine and rerun"
    );
}

// ast_getattr_fence — residual (d) of the 2026-11-05 OOM campaign
// (1648a72): an RSS-gated regression fence for OOM-campaign residual (a),
// the native `_ast` getattr path that materializes ~110 fresh class
// clones per attribute access OUTSIDE the GC heap (ast_bridge/TypeRef
// owner; the vendored `_ast.py` bulk-getattr shim landed in W114
// round-38/363600e avoids it, the core bug remains).
//
// The pre-fix leak OOMed this 64GB box at 9-44GB RSS on import-heavy
// probes. This suite runs the interpreter (target/debug/ap-run-probe,
// built by `cargo test -p ap-cli`) on a probe that does heavy native
// `_ast` getattr work (ast.parse of a 200-def module, then 1000 walks
// x getattr across the node class hierarchy) as a fenced subprocess:
//
//   * RSS cap   : AP_AST_FENCE_RSS_MB, default 512MB — measured HEAD
//                 peak is ~51MB (flat across the full 1000-iteration
//                 run; the pre-fix leak grew unbounded), so 512MB is a
//                 10x headroom tripwire.
//   * Deadline  : AP_AST_FENCE_DEADLINE_SECS, default 600s — measured
//                 HEAD wall time is ~170s on the debug build.
//   * Sampling  : `ps -o rss=` every 100ms (same seam as the ap-run
//                 differential harness fence, dea6c03); breach = SIGKILL
//                 + distinctive assert.
//
// Fence self-test (fence_kill_path_is_live): a plain Rust child burns
// memory until the fence must fire (8MB cap vs a 4GB allocation target).
// A fence that cannot fire is not a fence — if the kill path ever goes
// green, the sampler or cap plumbing broke and this suite must go red.
//
// CI: runs as part of `cargo test --workspace` in .github/workflows/
// ci.yml (job `rust`). Env overrides keep it runnable on slower boxes:
// set AP_AST_FENCE_RSS_MB=0 to disable the cap (deadline still applies).

/// Dump probe source + run it under ap-run-probe as a fenced subprocess.
/// Returns (outcome, peak_rss_bytes). `cap_override_mb`/`deadline_override`
/// shrink the fence for the kill-path self-test.
fn run_fenced(
    cap_override_mb: Option<u64>,
    deadline_override: Option<u64>,
) -> (Result<(std::process::Output, i32), String>, u64) {
    let rss_cap_mb = cap_override_mb
        .or_else(|| {
            std::env::var("AP_AST_FENCE_RSS_MB")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(512);
    let deadline_secs = deadline_override
        .or_else(|| {
            std::env::var("AP_AST_FENCE_DEADLINE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(600);

    let src = include_str!("../../../plans/robustness-ci/probe_ast_getattr.py");
    let dir = std::env::temp_dir().join(format!(
        "apx_ast_getattr_probe_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("ast_getattr_fence.rs:116")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("ast_getattr_fence.rs:121");
    let path = dir.join("probe_ast_getattr.py");
    std::fs::write(&path, src).expect("ast_getattr_fence.rs:123");

    let bin =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/ap-run-probe");
    let bin = if bin.exists() {
        bin
    } else {
        // Release build missing (developer ran only `cargo test`): fall back
        // to the debug binary but scale the deadline — debug frames make the
        // probe ~10x slower (measured 2026-11-06: 0.24s/iter release vs
        // 2.3s/iter debug on the W114 real-AST tree).
        let dbg = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug/ap-run-probe");
        eprintln!(
            "ast_getattr_fence: release probe binary missing; using debug \
             (deadline x10 internally not applied — set AP_AST_FENCE_DEADLINE_SECS)"
        );
        dbg
    };

    let mut child = std::process::Command::new(&bin)
        .arg(&path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn ap-run-probe");

    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    let mut peak_rss_bytes: u64 = 0;
    let outcome = loop {
        match child.try_wait().expect("wait probe") {
            Some(status) => {
                if let Some(kb) = rss_kb_of(pid) {
                    let b = kb.saturating_mul(1024);
                    if b > peak_rss_bytes {
                        peak_rss_bytes = b;
                    }
                }
                let out = child.wait_with_output().expect("collect probe output");
                break Ok((out, status.code().unwrap_or(-1)));
            }
            None => {
                if let Some(kb) = rss_kb_of(pid) {
                    let b = kb.saturating_mul(1024);
                    if b > peak_rss_bytes {
                        peak_rss_bytes = b;
                    }
                    if rss_cap_mb > 0 && b > rss_cap_mb * 1024 * 1024 {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(format!(
                            "memory: probe RSS reached {}MB, over the {}MB fence",
                            b / (1024 * 1024),
                            rss_cap_mb
                        ));
                    }
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "deadline: probe exceeded {deadline_secs}s (hang or pathological growth)"
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    (outcome, peak_rss_bytes)
}

/// Burn memory until killed: allocation climbs to a 512 MiB self-guard
/// (64x the 8MB self-test cap) and then HOLDS — the fence must SIGKILL the
/// child before the guard; an accidental UNFENCED hog (fence broken) peaks
/// at 512MB and spins visibly instead of exiting like a passing test.
/// Never raise this back toward gigabytes: this child can run without a
/// fence when invoked manually.
fn memory_hog() {
    // Fence-perf fix 2026-11-06: the old shape (1MB extends to a 256MB
    // HOG-SURVIVED exit) could FINISH before the parent's 100ms RSS sample
    // tick on a release build (~30ms total) — the fence never sampled a
    // breach and the kill-path self-test went red. New shape: allocate 16MB
    // chunks up to a 512MB self-guard, then HOLD the memory and spin until
    // the fence SIGKILLs us. An unfenced manual run now tops out at 512MB
    // instead of exiting early — the fence must fire; if the sampler/cap
    // plumbing is broken the hog spins (visible, bounded) rather than
    // passing silently.
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut c = vec![0u8; 16 << 20];
        // Commit EVERY page: vec![0u8; N] is calloc-backed (zero pages stay
        // uncommitted — RSS never crosses the cap and the sampler misses the
        // breach; observed on release 2026-11-06). Writing one byte per 16MB
        // chunk commits one PAGE only — same problem. A full pass commits all.
        for b in c.iter_mut() {
            *b = 1;
        }
        chunks.push(c);
        if chunks.len() * 16 >= 512 {
            // Self-guard reached: hold and wait for the fence (or a human).
            loop {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Fenced-subprocess driver shared by both tests: spawns a prepared
/// Command, applies the (cap, deadline) fence, returns outcome + peak.
fn fenced_child_cmd(
    cmd: &mut std::process::Command,
    cap_mb: u64,
    deadline_secs: u64,
) -> (Result<(std::process::Output, i32), String>, u64) {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("spawn fenced child");

    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    let mut peak_rss_bytes: u64 = 0;
    let outcome = loop {
        match child.try_wait().expect("wait fenced child") {
            Some(status) => {
                if let Some(kb) = rss_kb_of(pid) {
                    let b = kb.saturating_mul(1024);
                    if b > peak_rss_bytes {
                        peak_rss_bytes = b;
                    }
                }
                let out = child.wait_with_output().expect("collect fenced child");
                break Ok((out, status.code().unwrap_or(-1)));
            }
            None => {
                if let Some(kb) = rss_kb_of(pid) {
                    let b = kb.saturating_mul(1024);
                    if b > peak_rss_bytes {
                        peak_rss_bytes = b;
                    }
                    if cap_mb > 0 && b > cap_mb * 1024 * 1024 {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(format!(
                            "memory: child RSS reached {}MB, over the {}MB fence",
                            b / (1024 * 1024),
                            cap_mb
                        ));
                    }
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "deadline: child exceeded {deadline_secs}s (hang or pathological growth)"
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    (outcome, peak_rss_bytes)
}

#[test]
fn ast_getattr_leak_fence() {
    interpreter_gate();
    preflight_or_panic();
    let rss_cap_mb: u64 = std::env::var("AP_AST_FENCE_RSS_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512);
    let _deadline_secs: u64 = std::env::var("AP_AST_FENCE_DEADLINE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);

    let (outcome, peak) = run_fenced(None, None);
    match outcome {
        Ok((out, code)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(code, 0, "probe exited {code}; stderr:\n{stderr}");
            assert!(
                stdout.contains("FIELDS-TOTAL"),
                "probe did not reach its completion marker; stdout:\n{stdout}"
            );
            let cap = rss_cap_mb * 1024 * 1024;
            assert!(
                peak <= cap,
                "probe completed but peak RSS {}MB exceeds the {}MB fence (leak regression)",
                peak / (1024 * 1024),
                rss_cap_mb
            );
            println!(
                "ok: peak RSS {}MB <= {}MB cap; stdout marker present",
                peak / (1024 * 1024),
                rss_cap_mb
            );
        }
        Err(why) => panic!("ap-run-probe killed by fence: {why}"),
    }
}

#[test]
fn fence_kill_path_is_live() {
    // HOG MODE must be checked FIRST, before any lock acquisition: when
    // re-executed BY this test (below), the child must burn memory
    // immediately — taking the interpreter gate here would deadlock against
    // the parent's held flock (2026-11-06 harness review follow-up).
    if std::env::var("AP_AST_FENCE_HOG").is_ok() {
        memory_hog();
        return;
    }
    // 2026-11-06 harness review: single-tenancy + preflight (the hog child
    // is itself fenced at 8MB, so the footprint is bounded regardless).
    interpreter_gate();
    preflight_or_panic();
    // HOG MODE: when re-executed BY this test (below), the child detects
    // AP_AST_FENCE_HOG=1 and burns memory instead of running the suite.
    // This gate is what makes the re-exec SAFE: without it the child would
    // run the full test suite INCLUDING this very test, re-spawning
    // exponentially (a fork bomb — this is exactly what happened on
    // 2026-11-06 when the self-test ran unfixed).
    // The hog is this test binary re-executed with HOG env + a test filter,
    // so the child runs ONLY this test (which hogs and is fenced). The fence
    // must SIGKILL it before it reaches the (now modest) 256MB hog target.
    let hog = std::env::current_exe().expect("current_exe");
    let mut hog_cmd = std::process::Command::new(&hog);
    hog_cmd
        .args(["fence_kill_path_is_live", "--exact"])
        .env("AP_AST_FENCE_HOG", "1");
    let (outcome, _peak) = fenced_child_cmd(&mut hog_cmd, 8, 120);
    match outcome {
        Ok((out, _)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            panic!(
                "FENCE DID NOT FIRE: hog survived at cap=8MB (stdout: {stdout}) — \
                 sampler or cap plumbing is broken"
            );
        }
        Err(why) => {
            assert!(
                why.starts_with("memory:"),
                "expected the MEMORY arm to fire, got: {why}"
            );
            println!("ok: kill path fired ({why})");
        }
    }
}

/// Resident set size of `pid` in KiB via `ps -o rss=` (macOS + Linux).
fn rss_kb_of(pid: u32) -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    text.parse().ok()
}
