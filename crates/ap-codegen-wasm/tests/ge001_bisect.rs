// 03.04e bisect: run GE001 statement slices one per env pick.
#[test]
fn ge001_bisect() {
    let cases: &[(&str, &str)] = &[
        ("b1", "def g(n):\n    i = 0\n    while i < n:\n        yield i\n        i = i + 1\n    return 42\nit = g(3)\nprint(next(it))\nprint(next(it))\nprint(next(it))\n"),
        ("b2", "def g(n):\n    i = 0\n    while i < n:\n        yield i\n        i = i + 1\n    return 42\nit = g(3)\nprint(next(it))\nprint(next(it))\nprint(next(it))\nprint(next(it, 'exhausted'))\n"),
        ("b3", "def g(n):\n    i = 0\n    while i < n:\n        yield i\n        i = i + 1\n    return 42\nfor v in g(2):\n    print('v', v)\n"),
        ("b4", "def once():\n    yield 1\n    yield 2\ne = once()\nprint(next(e), next(e))\n"),
        ("b5", "def once():\n    yield 1\n    yield 2\ne = once()\nprint(next(e), next(e))\nprint(next(e, 'x'))\n"),
        ("b5a", "def once():\n    yield 1\n    yield 2\ne = once()\nprint(next(e, 'q'))\n"),
        ("b5b", "def once():\n    yield 1\n    yield 2\ne = once()\nprint(next(e))\nprint(next(e, 'x'))\n"),
        ("b5c", "def once():\n    yield 1\nprint(next(once(), 'q'))\n"),
        ("b5d", "def once():\n    yield 1\n    yield 2\ne = once()\nnext(e)\nnext(e)\nprint(next(e, 'x'))\n"),
        ("b5e", "def once():\n    yield 1\n    yield 2\ne = once()\nprint(next(e), next(e))\nprint('mid')\nprint(next(e, 'x'))\n"),
        ("b6", "def once():\n    yield 1\n    yield 2\ne2 = once()\nprint(next(e2))\ne2.close()\nprint(next(e2, 'after-close'))\n"),
        ("b6a", "def once():\n    yield 1\ne2 = once()\nprint(next(e2))\ne2.close()\nprint('closed')\n"),
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
        match ap_host::run(&bytes, 256 * 1024) {
            Ok(out) => {
                println!("stdout {:?}", out.stdout);
                println!("exit {}", out.exit_code);
            }
            Err(e) => println!("HOST-ERR {e:?}"),
        }
    }
}
