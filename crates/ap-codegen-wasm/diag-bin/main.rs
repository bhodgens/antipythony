//! diag: dump all operators of body index N (arg2), or the body containing offset (arg1); then validate the module
fn main() {
    let bytes = std::fs::read("/tmp/dbg3.wasm").unwrap();
    let args: Vec<String> = std::env::args().collect();
    let target: u64 = args
        .get(1)
        .map(|s| {
            u64::from_str_radix(s.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| s.parse().unwrap())
        })
        .unwrap_or(0xe84);
    let want_body: Option<u32> = args.get(2).and_then(|s| s.parse().ok());
    use wasmtime::wasmparser::{Operator, Payload};
    let mut i = 0u32;
    let mut found = false;
    for payload in wasmtime::wasmparser::Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::CodeSectionEntry(f) => {
                let a = f.range().start as u64;
                let dump = want_body == Some(i)
                    || (want_body.is_none() && a <= target && target <= f.range().end as u64);
                if dump {
                    found = true;
                    println!("body {} range {}..{}", i, a, f.range().end);
                    let mut locs = f.get_locals_reader().unwrap();
                    let mut idx = 0u32;
                    loop {
                        match locs.read() {
                            Ok((n, t)) => {
                                println!("  local {}..{} type {:?}", idx, idx + n, t);
                                idx += n;
                            }
                            Err(_) => break,
                        }
                    }
                }
                let mut ops = f.get_operators_reader().unwrap();
                loop {
                    let (op, off) = match ops.read_with_offset().map(|(o,p)| (o, p as u64)) {
                        Ok(p) => p,
                        Err(e) => {
                            if dump {
                                println!("READ-ERR {} at {}", e, ops.original_position());
                            }
                            break;
                        }
                    };
                    if dump {
                        println!("  rel{} @{} {:?}", off - a, off, op);
                    }
                    if ops.eof() {
                        break;
                    }
                }
                i += 1;
            }
            _ => {}
        }
    }
    if !found {
        println!("NO BODY FOUND (target={} want_body={:?})", target, want_body);
    }
    println!("total bodies: {}", i);
    // dump function-section type indices
    let mut fi = 0u32;
    for payload in wasmtime::wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmtime::wasmparser::Payload::FunctionSection(fs) = payload.unwrap() {
            for fty in fs {
                let t = fty.unwrap();
                if (2..40).contains(&fi) {
                    println!("funcidx {} type {}", fi, t);
                }
                fi += 1;
            }
        }
    }
    // dump global section types
    let mut gi = 0u32;
    for payload in wasmtime::wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmtime::wasmparser::Payload::GlobalSection(g) = payload.unwrap() {
            for gl in g {
                let gl = gl.unwrap();
                println!("global {} type {:?}", gi, gl.ty.content_type);
                gi += 1;
            }
        }
    }
    // dump rt_init (body 0) when DUMP_B0 is set
    if std::env::var("DUMP_B0").is_ok() {
        let mut j = 0u32;
        for payload in wasmtime::wasmparser::Parser::new(0).parse_all(&bytes) {
            if let wasmtime::wasmparser::Payload::CodeSectionEntry(f) = payload.unwrap() {
                if j
                    == std::env::var("DUMP_B0")
                        .ok()
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(0)
                {
                    let a = f.range().start as u64;
                    let mut ops = f.get_operators_reader().unwrap();
                    loop {
                        let (op, off) = match ops
                            .read_with_offset()
                            .map(|(o, p)| (o, p as u64))
                        {
                            Ok(p) => p,
                            Err(e) => {
                                println!("B0 READ-ERR {} at {}", e, ops.original_position());
                                break;
                            }
                        };
                        println!("  b0 rel{} @{} {:?}", off - a, off, op);
                        if ops.eof() {
                            break;
                        }
                    }
                }
                j += 1;
            }
        }
    }
    let mut feats = wasmtime::wasmparser::WasmFeatures::default();
    feats.set(wasmtime::wasmparser::WasmFeatures::GC, true);
    feats.set(
        wasmtime::wasmparser::WasmFeatures::FUNCTION_REFERENCES,
        true,
    );
    feats.set(wasmtime::wasmparser::WasmFeatures::EXCEPTIONS, true);
    feats.set(
        wasmtime::wasmparser::WasmFeatures::CUSTOM_PAGE_SIZES,
        true,
    );
    match wasmtime::wasmparser::Validator::new_with_features(feats).validate_all(&bytes) {
        Ok(_) => println!("MODULE VALIDATES"),
        Err(e) => println!("VALIDATE-ERR: {}", e),
    }
}
