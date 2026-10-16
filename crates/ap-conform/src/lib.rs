//! Differential conformance runner (spawns CPython).
//!
//! Runs each corpus script under a reference interpreter (`python3.14`, falling
//! back to `python3`) and under the implementation under test, then compares
//! stdout bytes, stderr bytes, and exit code.

use std::process::ExitStatus;

/// Outcome kind for a single corpus case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseOutcome {
    /// Expected and actual output matched byte-for-byte.
    Pass,
    /// Outputs differed.
    Fail,
    /// A process exceeded its timeout.
    Timeout,
    /// The harness could not run one or both sides (spawn error, missing
    /// interpreter).
    Error,
}

impl CaseOutcome {
    /// Stable lowercase tag used in JSON output.
    pub fn tag(self) -> &'static str {
        match self {
            CaseOutcome::Pass => "pass",
            CaseOutcome::Fail => "fail",
            CaseOutcome::Timeout => "timeout",
            CaseOutcome::Error => "error",
        }
    }
}

/// Captured result of running one interpreter on one script.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Raw stdout bytes.
    pub stdout: Vec<u8>,
    /// Raw stderr bytes.
    pub stderr: Vec<u8>,
    /// Process exit code, or `None` if killed by a signal.
    pub exit_code: Option<i32>,
}

/// One executed comparison: reference vs implementation.
#[derive(Debug, Clone)]
pub struct CaseResult {
    /// Path of the corpus script.
    pub case: String,
    /// Final outcome.
    pub outcome: CaseOutcome,
    /// Interpreter actually used for the reference side (e.g. `python3`).
    pub reference_interpreter: String,
    /// Reference-side result, if it ran.
    pub expected: Option<RunResult>,
    /// Implementation-side result, if it ran.
    pub actual: Option<RunResult>,
    /// Human-readable diff summary for failures.
    pub diff_summary: Option<String>,
}

impl CaseResult {
    /// True when the case passed.
    pub fn passed(&self) -> bool {
        self.outcome == CaseOutcome::Pass
    }
}

/// Expand a corpus glob into a sorted, deduplicated list of `.py` file paths.
pub fn expand_corpus(pattern: &str) -> std::io::Result<Vec<String>> {
    let mut files: Vec<String> = glob::glob(pattern)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?
        .filter_map(|entry| entry.ok())
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|p| p.ends_with(".py"))
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Locate the reference interpreter: `python3.14` first, then `python3`.
pub fn find_reference_interpreter() -> Option<String> {
    for candidate in ["python3.14", "python3"] {
        if which_exists(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn which_exists(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Run `program args[script]` capturing stdout/stderr/exit code, with timeout.
pub fn run_with_timeout(
    program: &str,
    args: &[String],
    script: &str,
    timeout: std::time::Duration,
) -> std::io::Result<RunResult> {
    use std::io::Read;
    use std::process::Stdio;

    let mut child = std::process::Command::new(program)
        .args(args)
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Pipe-draining threads so the child never blocks on a full pipe.
    let mut stdout_handle = child.stdout.take().expect("stdout piped");
    let mut stderr_handle = child.stderr.take().expect("stderr piped");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_handle.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_handle.read_to_end(&mut buf);
        buf
    });

    let deadline = std::time::Instant::now() + timeout;
    let status: ExitStatus = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Join anyway; killed children close their pipes.
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Ok(RunResult {
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        exit_code: None,
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    Ok(RunResult {
        stdout,
        stderr,
        exit_code: status.code(),
    })
}

/// Compare two runs and produce a [`CaseResult`].
pub fn compare_case(
    case: &str,
    expected: Option<RunResult>,
    actual: Option<RunResult>,
    expected_missing: bool,
    actual_missing: bool,
    reference_interpreter: &str,
) -> CaseResult {
    if expected_missing || actual_missing {
        return CaseResult {
            case: case.to_string(),
            outcome: CaseOutcome::Error,
            reference_interpreter: reference_interpreter.to_string(),
            expected,
            actual,
            diff_summary: Some(format!(
                "could not run: expected-side {} , actual-side {}",
                if expected_missing {
                    "unavailable"
                } else {
                    "ran"
                },
                if actual_missing { "unavailable" } else { "ran" }
            )),
        };
    }

    let (exp, act) = match (expected, actual) {
        (Some(e), Some(a)) => (e, a),
        _ => {
            return CaseResult {
                case: case.to_string(),
                outcome: CaseOutcome::Error,
                reference_interpreter: reference_interpreter.to_string(),
                expected: None,
                actual: None,
                diff_summary: Some("internal: missing run results".to_string()),
            };
        }
    };

    if exp.stdout == act.stdout && exp.stderr == act.stderr && exp.exit_code == act.exit_code {
        return CaseResult {
            case: case.to_string(),
            outcome: CaseOutcome::Pass,
            reference_interpreter: reference_interpreter.to_string(),
            expected: Some(exp),
            actual: Some(act),
            diff_summary: None,
        };
    }

    let mut parts = Vec::new();
    if exp.exit_code != act.exit_code {
        parts.push(format!(
            "exit code: expected {:?} != actual {:?}",
            exp.exit_code, act.exit_code
        ));
    }
    if exp.stdout != act.stdout {
        parts.push(diff_bytes("stdout", &exp.stdout, &act.stdout));
    }
    if exp.stderr != act.stderr {
        parts.push(diff_bytes("stderr", &exp.stderr, &act.stderr));
    }
    CaseResult {
        case: case.to_string(),
        outcome: CaseOutcome::Fail,
        reference_interpreter: reference_interpreter.to_string(),
        expected: Some(exp),
        actual: Some(act),
        diff_summary: Some(parts.join("; ")),
    }
}

fn diff_bytes(stream: &str, expected: &[u8], actual: &[u8]) -> String {
    let offset = expected
        .iter()
        .zip(actual.iter())
        .position(|(e, a)| e != a)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    format!(
        "{}: first differing byte at offset {} (expected len {}, actual len {})",
        stream,
        offset,
        expected.len(),
        actual.len()
    )
}

/// Run the full corpus against an implementation command.
pub fn run_corpus(
    files: &[String],
    impl_command: &[String],
    timeout: std::time::Duration,
) -> Result<Vec<CaseResult>, String> {
    if impl_command.is_empty() {
        return Err("empty impl command".to_string());
    }
    let reference = find_reference_interpreter().ok_or_else(|| {
        "no reference interpreter found (need python3.14 or python3 on PATH)".to_string()
    })?;

    let mut results = Vec::new();
    for file in files {
        let expected = run_with_timeout(&reference, &[], file, timeout);
        let actual = run_with_timeout(&impl_command[0], &impl_command[1..], file, timeout);

        let (expected_res, expected_missing) = match expected {
            Ok(r) if r.exit_code.is_none() => (Some(r), false),
            Ok(r) => (Some(r), false),
            Err(_) => (None, true),
        };
        let (actual_res, actual_missing) = match actual {
            Ok(r) => (Some(r), false),
            Err(_) => (None, true),
        };

        // Timeout is signalled by exit_code == None.
        let outcome_is_timeout =
            |r: &Option<RunResult>| matches!(r, Some(r) if r.exit_code.is_none());

        let result = if outcome_is_timeout(&expected_res) || outcome_is_timeout(&actual_res) {
            CaseResult {
                case: file.clone(),
                outcome: CaseOutcome::Timeout,
                reference_interpreter: reference.clone(),
                expected: expected_res,
                actual: actual_res,
                diff_summary: Some("process exceeded timeout and was killed".to_string()),
            }
        } else {
            compare_case(
                file,
                expected_res,
                actual_res,
                expected_missing,
                actual_missing,
                &reference,
            )
        };
        results.push(result);
    }
    Ok(results)
}
