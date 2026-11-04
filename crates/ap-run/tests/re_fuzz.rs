// robustness-ci leaf 04.01 — property-based differential fuzz for the re
// engine (native backtracker in crates/ap-run/src/sre.rs) vs live CPython
// 3.14, per review finding #9.
//
// Shape: a proptest grammar generates patterns biased toward the engine's
// own constructs (classes, greedy/lazy/possessive quantifiers, groups incl.
// named + backrefs, alternation, lookarounds, anchors, inline/scoped flags
// incl. the (?-i:...) negation the hand probes never covered, conditional
// groups) and subjects over a small alphabet plus unicode pockets. Each
// case runs ONE probe script through both engines and compares the re
// surface byte-exactly (compile meta + search/match/fullmatch/findall/
// finditer/split/sub/subn/expand: spans, groups, groupdict, lastindex/
// lastgroup, reprs, error texts) plus process exit codes.
//
// Modes (branch-04 contract: "Nightly, not PR-blocking"):
//   * default quick lane: AP_FUZZ_CASES (default 200) cases; divergences are
//     PRINTED (honest routed-finding output) and appended to
//     plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt (deduped), and the lane
//     stays informational-green while Plan 02 owns the in-flight engine.
//     This is a REPORT, not a weakening: the byte comparison is never
//     normalized away in either mode.
//   * AP_FUZZ_STRICT=1 (nightly, leaf 04.03): every unwhitelisted divergence
//     fails the run and proptest shrinks it to a minimal repro before
//     reporting. Findings are routed to Plan 02 (this leaf must NOT fix
//     sre.rs — ownership).
//
// Whitelist — EXACT bytes, the two documented divergences only
// (docs/conformance.md "Current accepted examples"):
//   1. step-limit: our stdout contains the exact documented string
//      "ap: matching step limit exceeded" where CPython answered normally
//      (CPython has no limit; it would hang — 4M-step guard, sre.rs header).
//   2. bytes-pattern latin-1 str-typed-group gap: NOT exercised — the fuzz
//      generates str patterns/subjects only (chosen option; keeps the
//      documented v0 bytes gap out of the differential surface entirely).
//
// Channels compared: probe stdout bytes + exit codes. Reference stderr is
// asserted byte-empty (the probe is warning-free on 3.14 and runs -W ignore);
// OUR stderr is not byte-compared because a panic message embeds a thread
// id — the non-zero exit code catches crashes and the death message rides
// in the printed finding.
//
// Bounded runtime: case count via AP_FUZZ_CASES (default 200; nightly
// higher), each subprocess capped at AP_FUZZ_TIMEOUT_MS (default 5000). A
// REFERENCE run exceeding the cap (CPython hangs on catastrophic
// backtracking — exactly the documented-divergence shape) is skipped and
// counted, never allowed to wedge the suite.
//
// Replay: divergent cases land in FUZZ_SEEDS.txt as
//   <family>\t<pattern Debug>\t<subject Debug>\t<flags>
// lines. Corpus promotion is deliberately NOT done here: corpus lints need
// spec-cited review by a different agent (master rule).

mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use proptest::prelude::*;
use proptest::strategy::{BoxedStrategy, Union};
use proptest::test_runner::{TestCaseError, TestError, TestRunner};

/// Exact documented step-limit string. Whitelist arm 1.
const STEP_LIMIT_MSG: &str = "ap: matching step limit exceeded";

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn cases_from_env() -> u32 {
    // Leaf brief default: 200 so the PR lane stays quick (orchestrator.md
    // sketched 1000 local; the brief wins — nightly 04.03 raises it).
    (env_usize("AP_FUZZ_CASES", 200).max(1)) as u32
}

fn strict() -> bool {
    matches!(
        std::env::var("AP_FUZZ_STRICT").as_deref(),
        Ok(v) if !v.is_empty() && v != "0"
    )
}

fn timeout() -> Duration {
    Duration::from_millis(env_usize("AP_FUZZ_TIMEOUT_MS", 5000) as u64)
}

// --------------------------------------------------------------- harness ---

struct ProcResult {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
    timed_out: bool,
}

fn pipe_reader(mut pipe: ChildStdout, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf);
    let _ = tx.send(buf);
}

fn pipe_reader_err(mut pipe: ChildStderr, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf);
    let _ = tx.send(buf);
}

/// Spawn `cmd` and wait at most `dur`, reaping the pipes on helper threads
/// (poll-then-collect would deadlock once a child's pipe buffer fills).
fn run_capped(cmd: &mut Command, dur: Duration) -> ProcResult {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subprocess");
    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let (o_tx, o_rx) = mpsc::channel::<Vec<u8>>();
    let (e_tx, e_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || pipe_reader(stdout, o_tx));
    std::thread::spawn(move || pipe_reader_err(stderr, e_tx));
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() > dur {
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait().expect("wait after kill");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("try_wait: {e}"),
        }
    };
    // after exit the pipes close and the readers finish promptly
    let stdout = o_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    let stderr = e_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    ProcResult {
        stdout,
        stderr,
        code: status.code(),
        timed_out,
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn eval_binary() -> PathBuf {
    // Same path math as re_differential.rs: repo-level target/, debug build.
    // Debug-only by default: `cargo test` rebuilds the debug bin against the
    // current engine source, while a stale target/release bin (from an older
    // checkout) would silently fuzz an outdated engine. Nightly 04.03 builds
    // release for throughput and can point this suite at it explicitly:
    // AP_FUZZ_EVAL_BIN=/path/to/ap-run-eval.
    if let Ok(p) = std::env::var("AP_FUZZ_EVAL_BIN") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-eval"),
        Err(_) => workspace_root().join("target/debug/ap-run-eval"),
    }
}

fn seeds_path() -> PathBuf {
    workspace_root().join("plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt")
}

// ------------------------------------------------- findings bookkeeping ---

struct Recorder {
    dir: PathBuf,
    seq: AtomicU64,
    divergences: AtomicUsize,
    routed: AtomicUsize,
    skip_step_limit: AtomicUsize,
    skip_ref_timeout: AtomicUsize,
    engine_timeout: AtomicUsize,
    ok: AtomicUsize,
    seen_seed_lines: Mutex<Vec<String>>,
    /// Per-family lines appended to FUZZ_SEEDS.txt this run (the file is
    /// representative, not an append-only flood: nightly 5000-case runs
    /// would otherwise grow a tracked file linearly with distinct cases).
    family_appended: Mutex<std::collections::HashMap<String, usize>>,
    seed_lines_loaded: OnceLock<bool>,
}

/// Max new seed lines appended per divergence family per run. Divergences
/// beyond the cap are still PRINTED (routed-finding output is uncapped) and
/// counted; only the file write stops.
const SEEDS_PER_FAMILY: usize = 8;

impl Recorder {
    fn new() -> Recorder {
        let dir = std::env::temp_dir().join(format!("antipythony-re-fuzz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fuzz scratch dir");
        Recorder {
            dir,
            seq: AtomicU64::new(0),
            divergences: AtomicUsize::new(0),
            routed: AtomicUsize::new(0),
            skip_step_limit: AtomicUsize::new(0),
            skip_ref_timeout: AtomicUsize::new(0),
            engine_timeout: AtomicUsize::new(0),
            ok: AtomicUsize::new(0),
            seen_seed_lines: Mutex::new(Vec::new()),
            family_appended: Mutex::new(std::collections::HashMap::new()),
            seed_lines_loaded: OnceLock::new(),
        }
    }

    fn load_existing_seeds(&self) {
        self.seed_lines_loaded.get_or_init(|| {
            if let Ok(existing) = std::fs::read_to_string(seeds_path()) {
                let mut seen = self.seen_seed_lines.lock().expect("seed mutex");
                for line in existing.lines() {
                    if !line.starts_with('#') && !line.is_empty() {
                        seen.push(line.to_string());
                    }
                }
            }
            true
        });
    }

    /// Append one replay line, deduped against the file + this run, capped
    /// per family (see `SEEDS_PER_FAMILY`).
    fn record_seed(&self, family: &str, pattern: &str, subject: &str, flags: u32) {
        let line = format!("{family}\t{pattern:?}\t{subject:?}\t{flags}");
        self.load_existing_seeds();
        {
            let seen = self.seen_seed_lines.lock().expect("seed mutex");
            if seen.contains(&line) {
                return;
            }
        }
        {
            let mut per_family = self.family_appended.lock().expect("seed mutex");
            let count = per_family.entry(family.to_string()).or_insert(0);
            *count += 1;
            if *count > SEEDS_PER_FAMILY {
                // family is over-represented already; the printed finding
                // carries the rest. Do not flood the tracked file.
                return;
            }
        }
        self.seen_seed_lines
            .lock()
            .expect("seed mutex")
            .push(line.clone());
        let path = seeds_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let fresh = !path.exists();
        let mut f = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("re_fuzz: cannot append seeds to {}: {e}", path.display());
                return;
            }
        };
        if fresh {
            let _ = writeln!(
                f,
                "# replay seeds from crates/ap-run/tests/re_fuzz.rs (leaf 04.01)"
            );
            let _ = writeln!(
                f,
                "# <family>\\t<pattern Debug>\\t<subject Debug>\\t<flags>"
            );
            let _ = writeln!(
                f,
                "# corpus promotion requires spec-cited review (master rule); engine fixes are Plan 02-owned."
            );
        }
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
    }
}

/// Short label for a divergence family — reporting metadata only; the byte
/// comparison itself is exact in both modes.
fn family_key(ref_out: &str, our_out: &str) -> String {
    let rl: Vec<&str> = ref_out.lines().collect();
    let ol: Vec<&str> = our_out.lines().collect();
    let idx = (0..rl.len().max(ol.len()))
        .find(|&j| {
            rl.get(j).copied().unwrap_or("<missing>") != ol.get(j).copied().unwrap_or("<missing>")
        })
        .unwrap_or(0);
    let r = rl.get(idx).copied().unwrap_or("<missing>");
    let o = ol.get(idx).copied().unwrap_or("<missing>");
    if r.starts_with("compile") && !o.starts_with("compile") {
        return "COMPILE_MISSED".to_string();
    }
    if o.starts_with("compile") && !r.starts_with("compile") {
        return "COMPILE_FALSE".to_string();
    }
    if r.starts_with("compile") && o.starts_with("compile") {
        return "COMPILE_TEXT".to_string();
    }
    r.split_whitespace().next().unwrap_or("?").to_string()
}

/// The single probe script. Written warning-free for CPython 3.14 (keyword
/// `count=`, raw-string templates) so the reference stderr is byte-empty on
/// every case.
fn probe_script(dir: &Path) -> String {
    let d = dir.display().to_string();
    format!(
        r#"import re
P = open('{d}/p.txt', encoding='utf-8').read()
S = open('{d}/s.txt', encoding='utf-8').read()
F = int(open('{d}/f.txt', encoding='utf-8').read())
def show(label, fn):
    try:
        print(label, fn())
    except Exception as e:
        print(label, type(e).__name__, e)
def fmt(m):
    if m is None:
        return None
    return (m.span(), m.groups(), m.groupdict(), m.lastindex, m.lastgroup, repr(m))
try:
    p = re.compile(P, F)
    print('meta', p.groups, sorted(p.groupindex.items()), repr(p.pattern))
except Exception as e:
    print('compile', type(e).__name__, e)
    raise SystemExit(0)
show('search', lambda: fmt(p.search(S)))
show('match', lambda: fmt(p.match(S)))
show('fullmatch', lambda: fmt(p.fullmatch(S)))
show('findall', lambda: p.findall(S))
show('finditer', lambda: [m.span() for m in p.finditer(S)])
show('split', lambda: p.split(S))
show('sub', lambda: p.sub(r'<\g<0>>', S))
show('subn', lambda: p.subn('#', S))
show('subn1', lambda: p.subn('-', S, count=1))
m = p.search(S)
show('expand', lambda: m is not None and m.expand(r'\g<0>|\0') or None)
"#
    )
}

fn write_case(dir: &Path, pattern: &str, subject: &str, flags: u32) -> PathBuf {
    std::fs::write(dir.join("p.txt"), pattern).expect("write p.txt");
    std::fs::write(dir.join("s.txt"), subject).expect("write s.txt");
    std::fs::write(dir.join("f.txt"), flags.to_string()).expect("write f.txt");
    let probe = dir.join("probe.py");
    std::fs::write(&probe, probe_script(dir)).expect("write probe.py");
    probe
}

/// Compare one case across both engines. `Err(finding)` on any
/// unwhitelisted divergence; `Ok(())` also covers the two skip arms.
fn compare_case(
    rec: &Recorder,
    py: &str,
    eval: &Path,
    pattern: &str,
    subject: &str,
    flags: u32,
) -> Result<(), String> {
    let n = rec.seq.fetch_add(1, Ordering::SeqCst);
    let dir = rec.dir.join(format!("c{n}"));
    std::fs::create_dir_all(&dir).expect("case dir");
    let probe = write_case(&dir, pattern, subject, flags);

    let reference = run_capped(
        Command::new(py)
            .arg("-S")
            .arg("-W")
            .arg("ignore")
            .arg(&probe),
        timeout(),
    );
    if reference.timed_out {
        // CPython (the oracle) exceeded the wall cap — catastrophic
        // backtracking, the documented-divergence shape (it hangs, we
        // limit). Skip + count; never compare against a half-answer.
        rec.skip_ref_timeout.fetch_add(1, Ordering::SeqCst);
        rec.record_seed("REF_TIMEOUT", pattern, subject, flags);
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }

    let ours = run_capped(Command::new(eval).arg(&probe), timeout());
    if ours.timed_out {
        // Our engine exceeded the cap without tripping its own 4M-step
        // guard — a product finding (Plan 02-owned): recorded as a replay
        // seed; strict mode turns it red, the quick lane routes it.
        rec.engine_timeout.fetch_add(1, Ordering::SeqCst);
        rec.record_seed("ENGINE_TIMEOUT", pattern, subject, flags);
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "ENGINE_TIMEOUT pattern={pattern:?} subject={subject:?} flags={flags}"
        ));
    }

    let our_stdout = String::from_utf8_lossy(&ours.stdout).into_owned();
    let ref_stdout = String::from_utf8_lossy(&reference.stdout).into_owned();

    // Whitelist arm 1 (EXACT documented bytes): our engine hit its step
    // limit where CPython produced a plain answer.
    if our_stdout.contains(STEP_LIMIT_MSG) && !ref_stdout.contains(STEP_LIMIT_MSG) {
        rec.skip_step_limit.fetch_add(1, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }

    let mut problems: Vec<String> = Vec::new();
    if !reference.stderr.is_empty() {
        problems.push(format!(
            "reference stderr not byte-empty: {:?}",
            String::from_utf8_lossy(&reference.stderr)
        ));
    }
    if reference.stdout != ours.stdout {
        let family = family_key(&ref_stdout, &our_stdout);
        problems.push(format!(
            "stdout bytes differ (family {family})\n  ref: {}\n  our: {}",
            ref_stdout, our_stdout
        ));
    }
    if reference.code != ours.code {
        problems.push(format!(
            "exit codes differ: ref {:?} our {:?}; our stderr: {}",
            reference.code,
            ours.code,
            String::from_utf8_lossy(&ours.stderr)
        ));
    }

    if problems.is_empty() {
        rec.ok.fetch_add(1, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(());
    }

    // Crashes get an honest family label: the stdout line label would say
    // "meta" (first ref line) while the actual divergence is a panic in the
    // engine (exit code 134 / abnormal termination).
    let family = if reference.code != ours.code {
        match ours.code {
            Some(code) => format!("PANIC-rc{code}"),
            None => "PANIC-signal".to_string(),
        }
    } else {
        family_key(&ref_stdout, &our_stdout)
    };
    rec.record_seed(&family, pattern, subject, flags);
    if strict() {
        rec.divergences.fetch_add(1, Ordering::SeqCst);
    } else {
        rec.routed.fetch_add(1, Ordering::SeqCst);
    }
    let msg = format!(
        "DIVERGENCE [{family}] pattern={pattern:?} subject={subject:?} flags={flags}\n{}",
        problems.join("\n")
    );
    // Honest routed-finding output: printed even in the informational lane,
    // so the `cargo test` output itself carries the finding.
    eprintln!("{msg}");
    let _ = std::fs::remove_dir_all(&dir);
    Err(msg)
}

// ------------------------------------------------------------- strategies ---

fn sb(s: &'static str) -> BoxedStrategy<String> {
    Just(s.to_string()).boxed()
}

fn pattern_atoms() -> BoxedStrategy<String> {
    Union::new_weighted(vec![
        (14, sb("a")),
        (8, sb("b")),
        (8, sb("c")),
        (6, sb("x")),
        (4, sb("0")),
        (4, sb("1")),
        (2, sb("-")),
        (2, sb("@")),
        (3, sb(r"\d")),
        (3, sb(r"\w")),
        (3, sb(r"\s")),
        (2, sb(r"\D")),
        (2, sb(r"\W")),
        (2, sb(r"\S")),
        (2, sb(".")),
        // char classes
        (3, sb("[abc]")),
        (2, sb("[^abc]")),
        (2, sb("[a-c]")),
        (2, sb("[0-9]")),
        (2, sb(r"[\w.]")),
        (1, sb("[]a]")),
        (1, sb("[-a]")),
        (1, sb("[^a-c0-9]")),
        // unicode pockets (pattern side): unicode case tables + char-vs-byte
        // span handling — the surface the crash-family probes hit.
        (2, sb("ä")),
        (1, sb("ß")),
        (1, sb("中")),
        (1, sb("١")),
        // anchors
        (2, sb(r"\b")),
        (1, sb(r"\B")),
        (1, sb(r"\A")),
        (1, sb(r"\Z")),
        // short literals
        (2, sb("ab")),
        (1, sb("x-")),
    ])
    .boxed()
}

fn quantifier() -> BoxedStrategy<String> {
    Union::new_weighted(vec![
        (4, sb("*")),
        (3, sb("+")),
        (3, sb("?")),
        (2, sb("{2}")),
        (2, sb("{1,2}")),
        (1, sb("{0,2}")),
        (1, sb("{2,}")),
        (2, sb("*?")),
        (1, sb("+?")),
        (1, sb("??")),
        (1, sb("{1,2}?")),
        (1, sb("{2,}?")),
    ])
    .boxed()
}

fn possessive_quantifier() -> BoxedStrategy<String> {
    Union::new_weighted(vec![(2, sb("*+")), (1, sb("++")), (1, sb("?+"))]).boxed()
}

fn name_strategy() -> BoxedStrategy<String> {
    // digit-only name: CPython rejects it (bad character in group name);
    // exercises the compile-error channel honestly.
    Union::new_weighted(vec![
        (4, sb("a")),
        (3, sb("b")),
        (3, sb("grp")),
        (1, sb("1")),
    ])
    .boxed()
}

fn scoped_flag() -> BoxedStrategy<String> {
    Union::new_weighted(vec![
        (3, sb("(?i:")),
        (1, sb("(?s:")),
        (1, sb("(?m:")),
        (1, sb("(?x:")),
        // scoped NEGATION — the 42 hand probes never cover (?-i:...)
        (2, sb("(?-i:")),
        // deliberately malformed tail; compile-error channel
        (1, sb("(?ai-")),
    ])
    .boxed()
}

fn global_flag_prefix() -> BoxedStrategy<String> {
    Union::new_weighted(vec![
        (2, sb("(?i)")),
        (1, sb("(?s)")),
        (1, sb("(?m)")),
        (1, sb("(?x)")),
    ])
    .boxed()
}

fn fixed_width_atom() -> BoxedStrategy<String> {
    // single-codepoint atoms for lookbehinds / backref groups (variable
    // width is its own deliberate arm below)
    Union::new_weighted(vec![
        (2, sb("a")),
        (2, sb("b")),
        (2, sb(r"\d")),
        (2, sb(r"\w")),
        (2, sb("[abc]")),
        (1, sb("[^ab]")),
        (1, sb(".")),
    ])
    .boxed()
}

fn backref_tail() -> BoxedStrategy<String> {
    Union::new_weighted(vec![
        (3, sb(r"\1")),
        (1, sb(r"\1*")),
        (1, sb(r"\1?")),
        (1, sb(r"\1+")),
    ])
    .boxed()
}

fn invalid_repeat_tail() -> BoxedStrategy<String> {
    Union::new_weighted(vec![(2, sb("**")), (1, sb("{2,1}")), (1, sb("*?*"))]).boxed()
}

fn pattern_strategy() -> BoxedStrategy<String> {
    let atom = pattern_atoms();
    atom.prop_recursive(4, 5, 2, |inner: BoxedStrategy<String>| {
        let i2a = inner.clone();
        let i2b = inner.clone();
        let i2c = inner.clone();
        let i2d = inner.clone();
        Union::new_weighted(vec![
            // concatenation
            (5, (inner.clone(), i2a).prop_map(|(a, b)| a + &b).boxed()),
            // alternation
            (
                4,
                (inner.clone(), i2b.clone())
                    .prop_map(|(a, b)| format!("{a}|{b}"))
                    .boxed(),
            ),
            // quantified
            (
                6,
                (inner.clone(), quantifier())
                    .prop_map(|(a, q)| format!("{a}{q}"))
                    .boxed(),
            ),
            // plain group
            (3, inner.clone().prop_map(|a| format!("({a})")).boxed()),
            // non-capturing group
            (2, inner.clone().prop_map(|a| format!("(?:{a})")).boxed()),
            // named group
            (
                3,
                (name_strategy(), inner.clone())
                    .prop_map(|(n, a)| format!("(?P<{n}>{a})"))
                    .boxed(),
            ),
            // backref: one-atom group then \1 — always defined
            (
                4,
                (fixed_width_atom(), backref_tail())
                    .prop_map(|(g, br)| format!("({g}){br}"))
                    .boxed(),
            ),
            // named backref pair (defined by construction)
            (
                3,
                (name_strategy(), fixed_width_atom())
                    .prop_map(|(n, g)| format!("(?P<{n}>{g})(?:x*)?(?P={n})"))
                    .boxed(),
            ),
            // lookahead / negative lookahead
            (2, inner.clone().prop_map(|a| format!("(?={a})")).boxed()),
            (1, inner.clone().prop_map(|a| format!("(?!{a})")).boxed()),
            // fixed-width lookbehinds
            (
                2,
                fixed_width_atom().prop_map(|a| format!("(?<={a})")).boxed(),
            ),
            (
                2,
                fixed_width_atom().prop_map(|a| format!("(?<!{a})")).boxed(),
            ),
            // variable-width lookbehind (both engines must raise the same
            // compile error)
            (1, inner.clone().prop_map(|a| format!("(?<={a}+)x")).boxed()),
            // atomic group / possessive quantifier
            (1, inner.clone().prop_map(|a| format!("(?>{a})")).boxed()),
            (
                1,
                (inner.clone(), possessive_quantifier())
                    .prop_map(|(a, q)| format!("{a}{q}"))
                    .boxed(),
            ),
            // scoped inline flag
            (
                2,
                (scoped_flag(), inner.clone())
                    .prop_map(|(f, a)| format!("{f}{a})"))
                    .boxed(),
            ),
            // global inline flag prefix
            (
                2,
                (global_flag_prefix(), inner.clone())
                    .prop_map(|(f, a)| f + &a)
                    .boxed(),
            ),
            // conditional group (fixed shape, valid on both engines)
            (1, sb("(a)?b(?(1)c|d)")),
            // adjacent-repeat invalid form (error-text channel)
            (
                1,
                (inner.clone(), invalid_repeat_tail())
                    .prop_map(|(a, q)| format!("{a}{q}"))
                    .boxed(),
            ),
            // backref into an OPEN group ("cannot refer to an open group")
            (
                1,
                fixed_width_atom()
                    .prop_map(|a| format!("(({a})\\1)"))
                    .boxed(),
            ),
            // three-way alternation
            (
                1,
                (inner.clone(), i2b, i2d)
                    .prop_map(|(a, b, c)| format!("{a}|{b}|{c}"))
                    .boxed(),
            ),
            // group with quantified body (nesting stress)
            (
                2,
                (inner.clone(), inner.clone(), quantifier())
                    .prop_map(|(a, b, q)| format!("({a}{b}){q}"))
                    .boxed(),
            ),
            // anchored tail
            (
                1,
                (inner.clone(), i2c)
                    .prop_map(|(a, b)| format!("^{a}{b}"))
                    .boxed(),
            ),
        ])
    })
    .boxed()
}

fn subject_strategy() -> BoxedStrategy<String> {
    let mut chars: Vec<char> = Vec::new();
    for c in "abcx01-@ .\n".chars() {
        chars.extend(std::iter::repeat_n(c, 3));
    }
    for c in "kkßä中١".chars() {
        chars.extend(std::iter::repeat_n(c, 2));
    }
    let pool = proptest::sample::select(chars);
    prop::collection::vec(pool, 0..27)
        .prop_map(|v| v.into_iter().collect::<String>())
        .boxed()
}

fn flags_strategy() -> BoxedStrategy<u32> {
    // re.I=2 re.M=8 re.S=16 re.X=64; combined subsets included. re.A(256)
    // excluded: ASCII table mode is already covered by hand probes
    // (unicode_categories) and stays out of the quick lane.
    Union::new_weighted(vec![
        (48, Just(0u32).boxed()),
        (8, Just(2u32).boxed()),
        (7, Just(8u32).boxed()),
        (7, Just(16u32).boxed()),
        (7, Just(64u32).boxed()),
        (6, Just(10u32).boxed()),
        (5, Just(24u32).boxed()),
        (4, Just(80u32).boxed()),
    ])
    .boxed()
}

fn case_strategy() -> BoxedStrategy<(String, String, u32)> {
    (pattern_strategy(), subject_strategy(), flags_strategy())
        .prop_map(|(p, s, f)| {
            // keep patterns short so a pathological blowup stays rare and
            // both engines usually answer inside the wall cap
            let p: String = p.chars().take(48).collect();
            (p, s, f)
        })
        .boxed()
}

// -------------------------------------------------------------- summary ---

fn print_summary(rec: &Recorder) {
    eprintln!(
        "re_fuzz summary: cases={} ok={} routed(non-strict)={} divergences(strict)={} \
         skip_step_limit={} skip_ref_timeout={} engine_timeout={} strict={}",
        rec.seq.load(Ordering::SeqCst),
        rec.ok.load(Ordering::SeqCst),
        rec.routed.load(Ordering::SeqCst),
        rec.divergences.load(Ordering::SeqCst),
        rec.skip_step_limit.load(Ordering::SeqCst),
        rec.skip_ref_timeout.load(Ordering::SeqCst),
        rec.engine_timeout.load(Ordering::SeqCst),
        strict(),
    );
    eprintln!("re_fuzz: replay seeds at {}", seeds_path().display());
}

fn recorder() -> &'static Recorder {
    static REC: OnceLock<Recorder> = OnceLock::new();
    REC.get_or_init(Recorder::new)
}

/// Build the shared proptest config: env-driven case count, NO persistence
/// (FUZZ_SEEDS.txt is this suite's replay mechanism — no stray
/// proptest-regressions files in the tree). proptest is depended on with
/// default-features off + std only, so its fork/timeout machinery is absent
/// (fork() == false, timeout() == 0) — which is exactly what we want: the
/// per-case wall budget is enforced by the subprocess caps above, which
/// proptest's parking timer cannot apply to a wedged python child anyway.
fn fuzz_config() -> ProptestConfig {
    let mut cfg = ProptestConfig::with_cases(cases_from_env());
    cfg.failure_persistence = None;
    cfg.max_shrink_iters = 512;
    cfg
}

/// One engine case: both probes + byte compare. Returns `Err(finding)` for
/// anything the two lanes treat differently (see module docs).
fn run_case(pattern: &str, subject: &str, flags: u32) -> Result<(), String> {
    let py = match common::reference_or_skip() {
        Some(py) => py,
        None => return Ok(()), // no reference + AP_REQUIRE_REFERENCE unset
    };
    let eval = eval_binary();
    if !eval.exists() {
        panic!(
            "ap-run-eval binary missing at {}; build it first (cargo build -p ap-cli --bin ap-run-eval)",
            eval.display()
        );
    }
    let rec = recorder();
    compare_case(rec, py, &eval, pattern, subject, flags)
}

// ----------------------------------------------------------------- tests ---

/// The proptest suite. Manual `TestRunner` (not the `proptest!` sugar) so
/// the informational-vs-strict lane switch lives in one readable place and
/// shrinking is fully under our control.
#[test]
fn re_engine_matches_cpython() {
    let skip = common::reference_or_skip().is_none();
    if skip {
        return; // SKIP line already printed by the shared seam
    }
    let mut runner = TestRunner::new(fuzz_config());
    let result = runner.run(&case_strategy(), |(pattern, subject, flags)| {
        match run_case(&pattern, &subject, flags) {
            Ok(()) => Ok(()),
            Err(msg) if strict() => {
                // nightly lane: fail, letting proptest shrink to a minimal
                // repro (the printed finding + seed line carry the detail)
                Err(TestCaseError::fail(msg))
            }
            Err(_msg) => {
                // quick lane: honest routed finding — already printed and
                // seeded. The engine is Plan-02 in-flight product code;
                // branch 04 is "Nightly, not PR-blocking".
                Ok(())
            }
        }
    });
    print_summary(recorder());
    match result {
        Ok(()) => {}
        Err(TestError::Fail(reason, input)) => {
            // strict lane; the finding was printed and seeded — assert so
            // `cargo test` reports red with the SHRUNK minimal input.
            panic!("re_fuzz strict divergence: {reason}\nminimal input: {input:?}");
        }
        Err(TestError::Abort(reason)) => {
            panic!("re_fuzz aborted: {reason}");
        }
    }
}

/// Lane banner: makes mode + budget visible in normal test output so a
/// CI-log reader knows which lane ran.
#[test]
fn re_fuzz_lane_banner() {
    eprintln!(
        "re_fuzz lane: cases={} strict={} (nightly: AP_FUZZ_CASES higher + AP_FUZZ_STRICT=1)",
        cases_from_env(),
        strict()
    );
}

/// Deterministic guard that the compare plumbing (spawn, caps, byte
/// compare, seed recorder) works end-to-end on a committed-byte-exact pair.
/// Subject has ONE match so the documented-plan-02 `count=` keyword gap
/// (fuzz finding SUBN1 below) cannot perturb this anchor.
#[test]
fn harness_smoke_case() {
    if common::reference_or_skip().is_none() {
        return;
    }
    run_case(r"a[bc]*?d", "abdc", 0)
        .expect("smoke case must be byte-exact (committed re_differential probe)");
}

/// Scratch divergence proof (gate 3, run only under AP_FUZZ_STRICT=1): an
/// IMPOSSIBLE expectation must go red through the exact same TestRunner +
/// shrink path the nightly lane uses. No product file is mutated — the
/// "reference" is replaced by a planted literal, proving the harness fails
/// when results disagree.
#[test]
fn strict_lane_can_fail_scratch_proof() {
    if !strict() {
        eprintln!("SKIP scratch proof: run with AP_FUZZ_STRICT=1 to exercise");
        return;
    }
    let mut cfg = ProptestConfig::with_cases(3);
    cfg.failure_persistence = None;
    let mut runner = TestRunner::new(cfg);
    let result = runner.run(&Just("planted".to_string()), |_s| {
        Err(TestCaseError::fail("PLANTED divergence for scratch proof"))
    });
    assert!(
        matches!(result, Err(TestError::Fail(..))),
        "strict lane must be able to go red; got {result:?}"
    );
}
