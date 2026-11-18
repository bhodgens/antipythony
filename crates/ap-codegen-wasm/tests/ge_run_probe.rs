// Throwaway 03.04d probe: run a source through parse->lower->ap-host and
// print the triple. SRC env var picks the source string (indexed).
#[test]
fn ge_run_probe() {
    let cases: &[(&str, &str)] = &[
        ("drain-list-lit", "print(list([1, 2]))\n"),
        ("drain-tuple-lit", "print(list((1, 2)))\n"),
        ("sum-min", "print(sum(range(4)))\n"),
        ("drain-gen-lit", "it = iter([5, 6])\nprint(next(it))\nprint(list(it))\n"),
        (
            "gen-min",
            "def g(n):\n    i = 0\n    while i < n:\n        yield i\n        i = i + 1\nit = g(3)\nprint(next(it))\nprint(next(it))\nprint(next(it, 'ex'))\n",
        ),
        (
            "comp-min",
            "squares = [x * x for x in range(5)]\nprint(squares)\n",
        ),
        (
            "dictcomp-min",
            "d = {k: k * 2 for k in range(3)}\nprint(d)\n",
        ),
        (
            "genexp-min",
            "n = sum(x for x in range(5))\nprint(n)\n",
        ),
        (
            "match-min",
            "m = [1, 2, 3]\nmatch m:\n    case [a, b, c]:\n        print('seq3', a, b, c)\n    case _:\n        print('no')\nmatch 7:\n    case 1 | 2:\n        print('or12')\n    case x if x > 5:\n        print('guard', x)\nmatch {'k': 5}:\n    case {'k': v}:\n        print('map', v)\nmatch [1, 2, 3, 4]:\n    case [first, *rest]:\n        print('star', first, rest)\n",
        ),
    ];
    let pick = std::env::var("PROBE_CASE").unwrap_or_default();
    for (name, src) in cases {
        if !pick.is_empty() && *name != pick {
            continue;
        }
        println!("=== {name} ===");
        let (module, _) = match ap_parse::lower_source(src) {
            Ok(m) => m,
            Err(e) => {
                println!("PARSE-ERR {e:?}");
                continue;
            }
        };
        let bytes = match ap_codegen_wasm::lower_module_ex(&module, Some(name)) {
            Ok(b) => b,
            Err(e) => {
                println!("LOWER-ERR {e:?}");
                continue;
            }
        };
        if std::env::var("PROBE_DUMP").is_ok() {
            std::fs::write(format!("/tmp/{name}.wasm"), &bytes).unwrap();
        }
        {
            use wasmparser::Validator;
            let feats = wasmparser::WasmFeatures::default()
                | wasmparser::WasmFeatures::GC
                | wasmparser::WasmFeatures::FUNCTION_REFERENCES
                | wasmparser::WasmFeatures::EXCEPTIONS;
            if let Err(e) = Validator::new_with_features(feats).validate_all(&bytes) {
                println!("VALIDATE-ERR {e}");
                continue;
            }
        }
        if std::env::var("PROBE_DUMP").is_ok() {
            std::fs::write(format!("/tmp/{name}.wasm"), &bytes).unwrap();
        }
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                println!("stdout {:?}", out.stdout);
                println!("stderr {:?}", out.stderr);
                println!("exit {}", out.exit_code);
            }
            Err(e) => println!("HOST-ERR {e}"),
        }
    }
}
