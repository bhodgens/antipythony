//! `ap-conform` binary: differential conformance runner CLI.

use clap::{Parser, Subcommand};
use serde::Serialize;
use std::io::Write;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "ap-conform",
    about = "Differential conformance runner: compare an implementation against CPython",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the corpus against an implementation.
    Run {
        /// Glob pattern of corpus .py files (e.g. 'tests/corpus/*.py').
        #[arg(long)]
        corpus: String,
        /// Implementation command (shell-split), run as `<command> <file>`.
        #[arg(long, num_args = 1.., value_delimiter = ' ')]
        impl_: Vec<String>,
        /// Only run cases whose path contains this substring.
        #[arg(long)]
        filter: Option<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Per-process timeout in seconds.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Execution tier to run the implementation under: `0` or `adaptive`
        /// (C-31 / 02.03 gate: "ap-conform green on FULL v1 corpus under
        /// tier-0 (`--tier 0`)"). Forwarded verbatim to the impl command as
        /// `--tier <mode>`, per the T-06/T-07/T-08 seam contract
        /// `<prefix> --tier 0`. The reference side never receives it.
        #[arg(long, value_name = "MODE")]
        tier: Option<String>,
    /// 02.06: impl-string mode accepting a BUILT-BINARY producer CLI (the
    /// 02.06 gate runs the conformance suite against the FILE, not
    /// `antipythony run`). Value = the CLI command prefix that BUILDS
    /// (shell-split), e.g. `--impl-binary 'target/release/antipythony'`.
    /// Per corpus case: `<cli> build <case> -o <tmp>/conform-<n>` runs
    /// first; the BUILT binary then executes the case. `--impl` is
    /// ignored when this is set.
    #[arg(long, num_args = 1.., value_delimiter = ' ')]
    impl_binary: Vec<String>,
    },
}

#[derive(Serialize)]
struct JsonCase<'a> {
    case: &'a str,
    pass: bool,
    outcome: &'static str,
    expected: Option<JsonRun<'a>>,
    actual: Option<JsonRun<'a>>,
    diff: Option<&'a str>,
    reference_interpreter: &'a str,
}

#[derive(Serialize)]
struct JsonRun<'a> {
    stdout: &'a [u8],
    stderr: &'a [u8],
    exit_code: Option<i32>,
}

#[derive(Serialize)]
struct JsonSummary {
    total: usize,
    passed: usize,
    failed: usize,
    timeouts: usize,
    errors: usize,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            corpus,
            impl_,
            filter,
            json,
            timeout,
            tier,
            impl_binary,
        } => {
            if let Some(mode) = &tier {
                if mode != "0" && mode != "adaptive" {
                    eprintln!("ap-conform: error: --tier must be '0' or 'adaptive', got '{mode}'");
                    return std::process::ExitCode::from(2);
                }
            }
            run(
                corpus,
                impl_,
                filter,
                json,
                Duration::from_secs(timeout),
                tier,
                impl_binary,
            )
        }
    }
}

fn run(
    corpus: String,
    mut impl_: Vec<String>,
    filter: Option<String>,
    json: bool,
    timeout: Duration,
    tier: Option<String>,
    impl_binary: Vec<String>,
) -> std::process::ExitCode {
    let files = match ap_conform::expand_corpus(&corpus) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("ap-conform: error: cannot expand corpus glob '{corpus}': {err}");
            return std::process::ExitCode::from(2);
        }
    };

    let files: Vec<String> = match filter {
        Some(ref needle) => files
            .into_iter()
            .filter(|f| f.contains(needle.as_str()))
            .collect(),
        None => files,
    };

    if files.is_empty() {
        eprintln!("ap-conform: error: corpus glob matched no files: '{corpus}'");
        return std::process::ExitCode::from(2);
    }

    // 02.06 --impl-binary: per-case BUILD then run the BUILT binary. The
    // files list is already filtered; build each into a numbered temp path
    // (stable order — the nth case always maps to conform-<n>).
    if !impl_binary.is_empty() {
        let tmp = std::env::temp_dir().join(format!(
            "apconform-binary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let mut built_impls: Vec<Option<String>> = Vec::with_capacity(files.len());
        for (i, file) in files.iter().enumerate() {
            let out_path = tmp.join(format!("conform-{i}"));
            let mut cmd = std::process::Command::new(&impl_binary[0]);
            cmd.args(&impl_binary[1..]);
            cmd.arg("build").arg(file).arg("-o").arg(&out_path);
            let ok = match cmd.output() {
                Ok(o) if o.status.success() && out_path.exists() => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755));
                    }
                    true
                }
                Ok(o) => {
                    eprintln!(
                        "ap-conform: build refused {} (rc {:?}): {}",
                        file,
                        o.status.code(),
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                    false
                }
                Err(e) => {
                    eprintln!("ap-conform: build spawn failed for {file}: {e}");
                    false
                }
            };
            built_impls.push(ok.then(|| out_path.to_string_lossy().into_owned()));
        }
        // Run the corpus case-by-case with per-case impl commands (the
        // generic run_corpus takes ONE impl command — loop here instead).
        let reference = match ap_conform::find_reference_interpreter() {
            Some(r) => r,
            None => {
                eprintln!("ap-conform: error: no reference interpreter found");
                return std::process::ExitCode::from(2);
            }
        };
        let mut results = Vec::with_capacity(files.len());
        for (i, file) in files.iter().enumerate() {
            let expected = ap_conform::run_with_timeout(&reference, &[], file, timeout);
            let actual = match &built_impls[i] {
                Some(bin) => ap_conform::run_with_timeout(bin, &[], file, timeout),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "build refused",
                )),
            };
            let (expected_res, expected_missing) = match expected {
                Ok(r) => (Some(r), false),
                Err(_) => (None, true),
            };
            let (actual_res, actual_missing) = match actual {
                Ok(r) => (Some(r), false),
                Err(_) => (None, true),
            };
            let outcome_is_timeout =
                |r: &Option<ap_conform::RunResult>| matches!(r, Some(r) if r.exit_code.is_none());
            let result = if outcome_is_timeout(&expected_res) || outcome_is_timeout(&actual_res) {
                ap_conform::CaseResult {
                    case: file.clone(),
                    outcome: ap_conform::CaseOutcome::Timeout,
                    reference_interpreter: reference.clone(),
                    expected: expected_res,
                    actual: actual_res,
                    diff_summary: Some("process exceeded timeout and was killed".to_string()),
                }
            } else {
                ap_conform::compare_case(
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
        let _ = std::fs::remove_dir_all(&tmp);
        return emit(&results, json);
    }

    // C-31 / 02.03 gate seam: forward the tier to the implementation only.
    // The reference side (CPython) has no tiers, so it must not receive the
    // flag. Appending to impl_ yields `<impl> ... --tier <mode> <file>`,
    // matching the documented seam `<prefix> --tier 0` (T-06/T-07/T-08).
    if let Some(mode) = tier {
        impl_.extend(["--tier".to_string(), mode]);
    }

    let results = match ap_conform::run_corpus(&files, &impl_, timeout) {
        Ok(results) => results,
        Err(err) => {
            eprintln!("ap-conform: error: {err}");
            return std::process::ExitCode::from(2);
        }
    };
    emit(&results, json)
}

/// Shared output emission (text table or JSON) + the process exit code.
fn emit(results: &[ap_conform::CaseResult], json: bool) -> std::process::ExitCode {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if json {
        let cases: Vec<JsonCase> = results
            .iter()
            .map(|r| JsonCase {
                case: &r.case,
                pass: r.passed(),
                outcome: r.outcome.tag(),
                expected: r.expected.as_ref().map(|e| JsonRun {
                    stdout: &e.stdout,
                    stderr: &e.stderr,
                    exit_code: e.exit_code,
                }),
                actual: r.actual.as_ref().map(|a| JsonRun {
                    stdout: &a.stdout,
                    stderr: &a.stderr,
                    exit_code: a.exit_code,
                }),
                diff: r.diff_summary.as_deref(),
                reference_interpreter: &r.reference_interpreter,
            })
            .collect();
        let total = cases.len();
        let passed = cases.iter().filter(|c| c.pass).count();
        let failed = cases.iter().filter(|c| c.outcome == "fail").count();
        let timeouts = cases.iter().filter(|c| c.outcome == "timeout").count();
        let errors = cases.iter().filter(|c| c.outcome == "error").count();
        let doc = serde_json::json!({
            "cases": cases,
            "summary": JsonSummary {
                total,
                passed,
                failed,
                timeouts,
                errors,
            },
        });
        if serde_json::to_writer_pretty(&mut out, &doc).is_err() {
            return std::process::ExitCode::from(2);
        }
        let _ = writeln!(out);
    } else {
        let _ = writeln!(
            out,
            "ap-conform: reference interpreter: {}",
            results
                .first()
                .map(|r| r.reference_interpreter.as_str())
                .unwrap_or("?")
        );
        for r in results.iter() {
            match r.outcome {
                ap_conform::CaseOutcome::Pass => {
                    let _ = writeln!(out, "PASS  {}", r.case);
                }
                ap_conform::CaseOutcome::Fail => {
                    let _ = writeln!(
                        out,
                        "FAIL  {}  ({})",
                        r.case,
                        r.diff_summary.as_deref().unwrap_or("outputs differ")
                    );
                }
                ap_conform::CaseOutcome::Timeout => {
                    let _ = writeln!(out, "TIMEOUT  {}", r.case);
                }
                ap_conform::CaseOutcome::Error => {
                    let _ = writeln!(
                        out,
                        "ERROR  {}  ({})",
                        r.case,
                        r.diff_summary.as_deref().unwrap_or("harness error")
                    );
                }
            }
        }
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed()).count();
        let failed = results
            .iter()
            .filter(|r| r.outcome == ap_conform::CaseOutcome::Fail)
            .count();
        let timeouts = results
            .iter()
            .filter(|r| r.outcome == ap_conform::CaseOutcome::Timeout)
            .count();
        let errors = results
            .iter()
            .filter(|r| r.outcome == ap_conform::CaseOutcome::Error)
            .count();
        let _ = writeln!(
            out,
            "summary: {total} cases, {passed} passed, {failed} failed, {timeouts} timeouts, {errors} errors"
        );
    }

    let all_pass = results.iter().all(|r| r.passed());
    if all_pass {
        std::process::ExitCode::from(0)
    } else {
        std::process::ExitCode::from(1)
    }
}
