//! `ap-host` — the standalone wasm runner CLI (leaf 03.05).
//!
//! Shape (the form tests/t16_hybrid and 04.04's standalone runner cite —
//! keep the interface IDENTICAL downstream):
//!
//! ```text
//! ap-host run <module.wasm> [module-args...]
//!   --cap <bytes>        capture pipe bound (default 262144)
//!   --arg <a> ...        grant module args (repeatable; AFTER the module
//!                        they may also be passed bare)
//!   --env K=V ...        grant one env var (repeatable; DEFAULT DENY —
//!                        the host env is never inherited wholesale)
//!   --argv0 <name>       argv[0] presented to the module
//!   --ttfb               print the time-to-first-stdout-byte measurement
//!   --validate-only      surface checks + compile, no execution
//! ```
//!
//! Module args after the .wasm path are passed through verbatim; the flag
//! parser stops at the first non-flag token (the module path) or `--`.
//!
//! Exit codes: 0 module rc 0; else the MODULE's exit code when the module
//! ran (proc_exit), 126 surface/ABI refusal, 127 host/internal error.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::process::ExitCode;

use ap_host::{validate_module_surface, HostError, RunOptions};

const USAGE: &str = "\
ap-host run <module.wasm> [module-args...] [--]
  --cap <bytes>     capture pipe bound (default 262144)
  --arg <a>         grant one module arg (repeatable)
  --env K=V         grant one env var (repeatable; default deny)
  --argv0 <name>    argv[0] presented to the module
  --ttfb            print time-to-first-stdout-byte (us) to stderr
  --validate-only   surface checks + compile, no execution
  -h, --help        this text";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match cli_run(&raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(msg)) => {
            eprintln!("ap-host: {msg}\n\n{USAGE}");
            ExitCode::from(127)
        }
        Err(CliError::Host(e, rc)) => {
            eprintln!("ap-host: {e}");
            ExitCode::from(rc)
        }
    }
}

enum CliError {
    Usage(String),
    /// Host error with the exit code to die with.
    Host(HostError, u8),
}

/// Run-time context of the current capture mode — main() mirrors stdout to
/// the console as it drains; stderr always drains to the console.
struct Invocation {
    path: String,
    opts: RunOptions,
    ttfb: bool,
    validate_only: bool,
    module_args: Vec<String>,
}

fn cli_run(argv: &[String]) -> Result<(), CliError> {
    let mut inv = Invocation {
        path: String::new(),
        opts: RunOptions::new(),
        ttfb: false,
        validate_only: false,
        module_args: Vec::new(),
    };
    let mut it = argv.iter();
    let mut no_more_flags = false;
    while let Some(a) = it.next() {
        if !no_more_flags && a == "--" {
            no_more_flags = true;
            continue;
        }
        if !no_more_flags && a.starts_with('-') && a.len() > 1 {
            match a.as_str() {
                "-h" | "--help" => {
                    println!("{USAGE}");
                    return Ok(());
                }
                "--cap" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError::Usage("--cap needs a value".into()))?;
                    inv.opts.capture_cap = v
                        .parse::<usize>()
                        .map_err(|_| CliError::Usage(format!("--cap: {v} is not a byte count")))?;
                }
                "--arg" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError::Usage("--arg needs a value".into()))?;
                    inv.opts.args.push(v.clone());
                }
                "--env" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError::Usage("--env needs K=V".into()))?;
                    let (k, val) = v
                        .split_once('=')
                        .ok_or_else(|| CliError::Usage(format!("--env: {v:?} is not K=V")))?;
                    inv.opts.env.push((k.to_string(), val.to_string()));
                }
                "--argv0" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError::Usage("--argv0 needs a value".into()))?;
                    inv.opts.argv0 = Some(v.clone());
                }
                "--ttfb" => {
                    inv.ttfb = true;
                    inv.opts.measure_ttfb = true;
                }
                "--validate-only" => inv.validate_only = true,
                other => {
                    return Err(CliError::Usage(format!("unknown flag {other:?}")));
                }
            }
            continue;
        }
        if inv.path.is_empty() {
            inv.path = a.clone();
        } else {
            // Positional after the module path: a module arg.
            inv.module_args.push(a.clone());
        }
    }
    if inv.path.is_empty() {
        return Err(CliError::Usage("missing <module.wasm>".into()));
    }
    // Bare module args (positionals) ride the same grant as --arg.
    inv.opts.args.extend(inv.module_args.iter().cloned());

    let bytes = match std::fs::read(&inv.path) {
        Ok(b) => b,
        Err(e) => {
            return Err(CliError::Host(
                HostError::Wasm(format!("cannot read {}: {e}", inv.path)),
                127,
            ))
        }
    };

    if inv.validate_only {
        return match validate_module_surface(&bytes) {
            Ok(()) => {
                println!("ap-host: {} surface OK (ABI stamp accepted)", inv.path);
                Ok(())
            }
            Err(e) => Err(CliError::Host(e, 126)),
        };
    }

    let argv0 = inv.opts.argv0.clone().unwrap_or_else(|| {
        std::path::Path::new(&inv.path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| inv.path.clone())
    });
    inv.opts.argv0 = Some(argv0);

    match ap_host::run_with_options(&bytes, &inv.opts) {
        Ok(out) => {
            // Emit the captured streams verbatim (byte-exact contract).
            print!("{}", out.stdout);
            eprint!("{}", out.stderr);
            if inv.ttfb {
                match out.ttfb_us {
                    Some(us) => eprintln!(
                        "[ap-host] ttfb {}us ({}.{:03}ms), total {}us",
                        us,
                        us / 1000,
                        us % 1000,
                        out.total_us
                    ),
                    None => eprintln!("[ap-host] ttfb: module wrote nothing to stdout"),
                }
            }
            if out.exit_code == 0 {
                Ok(())
            } else {
                // The MODULE chose this exit code — propagate it verbatim
                // (byte/exit-exact contract), clamped into u8 range.
                Err(CliError::Host(
                    HostError::Wasm(String::new()),
                    out.exit_code.clamp(0, 255) as u8,
                ))
            }
        }
        Err(e @ HostError::Abi(_)) | Err(e @ HostError::Threading(_)) => {
            Err(CliError::Host(e, 126))
        }
        Err(e) => Err(CliError::Host(e, 127)),
    }
}
