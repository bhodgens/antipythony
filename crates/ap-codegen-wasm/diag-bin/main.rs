//! Dump operators of function N (by code-section body order) in a byte range.
fn main() {
    let case = std::env::args().nth(1).expect("case name");
    let func_no: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    let path = format!(
        "/Users/caimlas/git/antipythony/tests/corpus/values/{}.py",
        case
    );
    let src = std::fs::read_to_string(&path).unwrap();
    let (m, _) = ap_parse::lower_source(&src).unwrap();
    let bytes = ap_codegen_wasm::lower_module(&m).unwrap();
    std::fs::write("/tmp/diag.wasm", &bytes).unwrap();

    let lo = std::env::args()
        .nth(3)
        .and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let hi = std::env::args()
        .nth(4)
        .and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0xffff_ffff);

    use wasmparser::{Operator, Payload};
    let mut n_bodies = 0u64;
    let mut all_ops: Vec<Vec<(usize, String)>> = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(f) = payload.unwrap() {
            n_bodies += 1;
            let mut reader = f.get_operators_reader().unwrap();
            let mut ops: Vec<(usize, String)> = Vec::new();
            loop {
                let (op, off) = match reader.read_with_offset() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let is_end = matches!(op, Operator::End);
                ops.push((off as usize, format!("{:?}", op)));
                if is_end {
                    break;
                }
            }
            all_ops.push(ops);
        }
    }
    // print which body contains the target offset
    for (i, ops) in all_ops.iter().enumerate() {
        let first = ops.first().map(|o| o.0).unwrap_or(0);
        let last = ops.last().map(|o| o.0).unwrap_or(0);
        if true {
            println!(
                "## offset in body #{} (range 0x{:x}..0x{:x}, {} ops)",
                i + 1,
                first,
                last,
                ops.len()
            );
            for (off, txt) in ops {
                if *off >= lo && *off <= hi {
                    println!("0x{:04x}: {}", off, txt);
                }
            }
        }
    }
    println!("## total bodies {}", n_bodies);
}
