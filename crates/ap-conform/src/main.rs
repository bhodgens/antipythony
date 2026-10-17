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
        for r in &results {
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
