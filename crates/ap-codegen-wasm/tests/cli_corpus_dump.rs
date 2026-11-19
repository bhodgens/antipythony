//! Leaf 03.05 corpus-dump tool: lower every currently-green wasm corpus
//! case to a .wasm file so the ap-host CLI binary can run them WITHOUT a
//! cargo test harness. Diagnostic example (cargo test -p ap-codegen-wasm
//! --test cli_corpus_dump -- --nocapture); writes under
//! ~/.git/worktrees/leaf-0305/logs/wasm-corpus/ (never /tmp, machine
//! policy).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|p| p.to_path_buf())
        .expect("crate at <root>/crates/ap-codegen-wasm")
}

#[test]
fn dump_corpus_wasm() {
    let root = repo_root();
    let out_dir = root.join("logs/wasm-corpus");
    std::fs::create_dir_all(&out_dir).expect("mkdir logs/wasm-corpus");

    // Leaf 03.06: generators_wasm + match_wasm families appended (03.04
    // closed their lowering — 41 pinned cases total dump through here).
    let fams = [
        "controlflow_wasm",
        "classes_wasm",
        "exceptions_wasm",
        "generators_wasm",
        "match_wasm",
    ];
    let mut dumped = 0usize;
    for fam in fams {
        let dir = root.join("tests/corpus").join(fam);
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .expect("family dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .collect();
        paths.sort();
        for p in paths {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&p).unwrap();
            let (module, _) = ap_parse::lower_source(&src).expect("parse");
            let bytes = ap_codegen_wasm::lower_module_ex(&module, Some(&stem)).expect("lower");
            let out = out_dir.join(format!("{stem}.wasm"));
            std::fs::write(&out, &bytes).expect("write wasm");
            dumped += 1;
        }
    }
    // The drain probes (03.04c slice, currently green) as inline sources.
    let probes: &[(&str, &str)] = &[
        ("hello", "print('hello, wasm')\n"),
        (
            "for-range",
            "for j in range(2):\n    print('j', j)\nprint('done')\n",
        ),
        (
            "for-list",
            "for v in [10, 20, 30]:\n    print('v', v)\nprint('done')\n",
        ),
        (
            "for-tuple-else",
            "for w in (1, 2):\n    print('w', w)\nprint('done')\n",
        ),
        (
            "for-range-sum",
            "total = 0\nfor i in range(3):\n    total = total + i\nprint(total)\n",
        ),
        (
            "for-break",
            "for i in range(5):\n    if i == 2:\n        break\n    print(i)\nprint('end')\n",
        ),
        ("drain-list-lit", "print(list([1, 2]))\n"),
        ("drain-tuple-lit", "print(list((1, 2)))\n"),
        ("sum-min", "print(sum(range(4)))\n"),
        (
            "drain-gen-lit",
            "it = iter([5, 6])\nprint(next(it))\nprint(list(it))\n",
        ),
    ];
    for (name, src) in probes {
        let (module, _) = ap_parse::lower_source(src).expect("parse");
        let bytes = ap_codegen_wasm::lower_module_ex(&module, Some(name)).expect("lower");
        let out = out_dir.join(format!("{name}.wasm"));
        std::fs::write(&out, &bytes).expect("write wasm");
        dumped += 1;
    }
    println!("dumped {dumped} modules to {}", out_dir.display());
}
