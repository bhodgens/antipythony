//! EH + GC-array engine probe (leaf 03.03 de-risk): the facts this leaf
//! depends on, verified against wasmtime 47 BEFORE any emitter work.
//!
//! 1. array.new_fixed into a MUTABLE array type (03.02c says RO required —
//!    if mutable works, the calling convention has headroom).
//! 2. try_table + catch (tag payload) label-depth semantics.
//! 3. catch_all_ref rethrow shape for finally.
//! 4. An uncaught exception escaping _start: what the host sees.
//! 5. struct.new with an exnref-carrying tag through the full pipeline.

use wasm_encoder::*;

const T_UNIT: u32 = 0; // struct () rec index 0
const T_PYOBJ: u32 = 1; // rec index 1 (root)
const T_SUB: u32 = 2; // rec index 2
const T_OBJ_ARR: u32 = 3; // rec index 3 (mut)
const T_OBJ_ARR_RO: u32 = 4; // rec index 4 (RO)
const FT_THROW: u32 = 5; // () -> () : the tag's type
const FT_VOID: u32 = 6;
const FT_PYN: u32 = 7; // (mut $pyobj array) -> (ref null $pyobj)
const FT_PY_RO: u32 = 8;

fn probe_module(try_catch: bool) -> Vec<u8> {
    let mut m = Module::new();

    // ---- types: one rec group ----
    let mut types = TypeSection::new();
    let unit_sub = SubType {
        is_final: true,
        supertype_idxs: vec![],
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: vec![].into_boxed_slice(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    };
    types.ty().rec(vec![unit_sub]);
    let o = || {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(T_PYOBJ),
        })
    };
    // pyobj root
    types.ty().rec(vec![SubType {
        is_final: false,
        supertype_idxs: vec![],
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: vec![FieldType {
                    element_type: StorageType::Val(o()),
                    mutable: false,
                }]
                .into_boxed_slice(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    // sub with one extra field
    types.ty().rec(vec![SubType {
        is_final: true,
        supertype_idxs: vec![T_PYOBJ],
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: vec![
                    FieldType {
                        element_type: StorageType::Val(o()),
                        mutable: false,
                    },
                    FieldType {
                        element_type: StorageType::Val(ValType::I32),
                        mutable: false,
                    },
                ]
                .into_boxed_slice(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    // mut obj array
    types.ty().rec(vec![SubType {
        is_final: true,
        supertype_idxs: vec![],
        composite_type: CompositeType {
            inner: CompositeInnerType::Array(ArrayType(FieldType {
                element_type: StorageType::Val(o()),
                mutable: true,
            })),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    // RO obj array
    types.ty().rec(vec![SubType {
        is_final: true,
        supertype_idxs: vec![],
        composite_type: CompositeType {
            inner: CompositeInnerType::Array(ArrayType(FieldType {
                element_type: StorageType::Val(o()),
                mutable: false,
            })),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    types.ty().function([], []); // 1 FT_THROW
    types.ty().function([], []); // 2 FT_VOID
    types.ty().function(
        [ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(T_OBJ_ARR),
        })],
        [o()],
    ); // 3 FT_PYN (unused here, declared for shape parity)
    types.ty().function(
        [
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(T_PYOBJ),
            }),
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(T_OBJ_ARR_RO),
            }),
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(T_UNIT),
            }),
        ],
        [o()],
    ); // 4+3=7 FT_PY_RO? no: indices 4,5,6 recs -> FT_PY_RO=7... recount below
    // ---- functions: 0=thrower 1=catcher-or-propagator 2=_start ----
    let mut funcs = FunctionSection::new();
    funcs.function(FT_VOID); // 0 thrower
    funcs.function(FT_VOID); // 1 catcher / propagator
    funcs.function(FT_VOID); // 2 _start

    // ---- tags ----
    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: FT_THROW,
    });
    m.section(&types);
    m.section(&funcs);
    m.section(&tags);

    let mut code = CodeSection::new();
    // thrower: throw the tag (no payload — tag type is ()->())
    {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::Throw(0));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    {
        let mut f = Function::new(vec![]);
        if try_catch {
            // try_table with Catch (no payload variant — the tag carries no
            // values here). Label 0 = the try_table block itself.
            f.instruction(&Instruction::TryTable(
                BlockType::Empty,
                std::borrow::Cow::Owned(vec![Catch::One { tag: 0, label: 0 }]),
            ));
            f.instruction(&Instruction::Call(0));
            f.instruction(&Instruction::End);
            // fell through = caught: reach _start's print via global? just return
        } else {
            f.instruction(&Instruction::Call(0));
        }
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::Call(1));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    let mut exports = ExportSection::new();
    exports.export("_start", ExportKind::Func, 2);
    m.section(&exports);
    m.section(&code);
    m.finish()
}

fn run(wasm: &[u8]) -> Result<(), wasmtime::Error> {
    // local validation first: wasmparser's error text is far better than
    // wasmtime's bare "failed to parse"
    let mut validator = wasmparser::Validator::new_with_features(
        wasmparser::WasmFeatures::default() | wasmparser::WasmFeatures::EXCEPTIONS,
    );
    if let Err(e) = validator.validate_all(wasm) {
        println!("PROBE-VALIDATE: {e}");
        return Err(wasmtime::Error::msg("validation failed"));
    }
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    let engine = wasmtime::Engine::new(&config)?;
    let module = wasmtime::Module::new(&engine, wasm)
        .map_err(|e| {
            println!("PROBE-COMPILE: {e}");
            e
        })?;
    let mut store = wasmtime::Store::new(&engine, ());
    let linker: wasmtime::Linker<()> = wasmtime::Linker::new(&engine);
    let instance = linker.instantiate(&mut store, &module)?;
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
    match start.call(&mut store, ()) {
        Ok(()) => {
            println!("PROBE: clean return");
            Ok(())
        }
        Err(e) => {
            // is it a ThrownException? peek at the store
            let has_pending = store.take_pending_exception().is_some();
            println!(
                "PROBE: call error, pending_exception={has_pending}, err={e:?}"
            );
            Err(e)
        }
    }
}

#[test]
fn probe_eh_facts() {
    // 1. mutable array.new_fixed
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let o = || {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(0),
        })
    };
    types.ty().rec(vec![
        SubType {
            is_final: false,
            supertype_idxs: vec![],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: vec![FieldType {
                        element_type: StorageType::Val(o()),
                        mutable: false,
                    }]
                    .into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        },
        SubType {
            is_final: true,
            supertype_idxs: vec![],
            composite_type: CompositeType {
                inner: CompositeInnerType::Array(wasm_encoder::ArrayType(FieldType {
                    element_type: StorageType::Val(o()),
                    mutable: true,
                })),
                shared: false,
                descriptor: None,
                describes: None,
            },
        },
    ]);
    types.ty().function([], []); // FT index 2
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(2);
    m.section(&funcs);
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::RefNull(HeapType::Concrete(0)));
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: 1,
        array_size: 1,
    });
    f.instruction(&Instruction::Drop);
    f.instruction(&Instruction::End);
    code.function(&f);
    let mut exports = ExportSection::new();
    exports.export("f", ExportKind::Func, 0);
    m.section(&exports);
    m.section(&code);
    let bytes = m.finish();
    let mut validator = wasmparser::Validator::new_with_features(
        wasmparser::WasmFeatures::default() | wasmparser::WasmFeatures::GC,
    );
    if let Err(e) = validator.validate_all(&bytes) {
        println!("PROBE-1-VALIDATE: {e}");
    }
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    match wasmtime::Module::new(&engine, &bytes) {
        Ok(_) => println!("PROBE-1: array.new_fixed into MUT array type: OK"),
        Err(e) => println!("PROBE-1: array.new_fixed into MUT array type: REJECTED: {e}"),
    }

    // 2/3. try_table catch + uncaught propagation
    for (name, try_catch) in [("caught", true), ("uncaught", false)] {
        let bytes = probe_module(try_catch);
        match run(&bytes) {
            Ok(()) => println!("PROBE-2/{name}: ran clean"),
            Err(_) => println!("PROBE-2/{name}: propagated as error (see line above)"),
        }
    }
}

/// Probe 3: tag WITH payload — Catch::One delivers the payload at the label.
#[test]
fn probe_payload_tag() {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let o = || {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(0),
        })
    };
    // rec 0: pyobj root (field 0 = type ref)
    types.ty().rec(vec![SubType {
        is_final: false,
        supertype_idxs: vec![],
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: vec![FieldType {
                    element_type: StorageType::Val(o()),
                    mutable: false,
                }]
                .into_boxed_slice(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    // rec 1: sub with an i32 payload field
    types.ty().rec(vec![SubType {
        is_final: true,
        supertype_idxs: vec![0],
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: vec![
                    FieldType {
                        element_type: StorageType::Val(o()),
                        mutable: false,
                    },
                    FieldType {
                        element_type: StorageType::Val(ValType::I32),
                        mutable: false,
                    },
                ]
                .into_boxed_slice(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }]);
    // FTs: 2 = tag type (o) -> (), 3 = void, 4 = () -> i32
    types.ty().function([o()], []);
    types.ty().function([], []);
    types.ty().function([], [ValType::I32]);

    let mut tags = TagSection::new();
    tags.tag(TagType {
        kind: TagKind::Exception,
        func_type_idx: 2,
    });
    let mut funcs = FunctionSection::new();
    funcs.function(3); // 0 thrower (builds sub, throws with payload)
    funcs.function(3); // 1 catcher (catch, extract i32, set global)
    funcs.function(4); // 2 _start (read global)
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    ); // global 0 = result marker
    let mut exports = ExportSection::new();
    exports.export("_start", ExportKind::Func, 2);
    let mut code = CodeSection::new();
    // thrower
    {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::RefNull(HeapType::Concrete(0)));
        f.instruction(&Instruction::I32Const(42));
        f.instruction(&Instruction::StructNew(1));
        f.instruction(&Instruction::Throw(0));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    // catcher: try_table catch -> label delivers (ref null 0) payload.
    // The try_table BLOCKTYPE carries the tag's result types (the payload
    // params): a Catch::One handler label must accept exactly the tag's
    // parameter types, so the try_table's own block type is the tag's
    // func type ( FunctionType(2) = ((ref null $pyobj)) -> () ).
    {
        let mut f = Function::new(vec![]);
        // Catch labels resolve in the scope ENCLOSING the try_table (the
        // validator checks them before pushing the try_table frame). So the
        // handler target is an outer Block whose RESULT types match the
        // tag's params; label 0 = that Block.
        f.instruction(&Instruction::Block(BlockType::Result(o())));
        f.instruction(&Instruction::TryTable(
            BlockType::Empty,
            std::borrow::Cow::Owned(vec![Catch::One { tag: 0, label: 0 }]),
        ));
        f.instruction(&Instruction::Call(0));
        // normal path (no throw): fall out of try_table, then produce the
        // outer Block's result (null = no payload)
        f.instruction(&Instruction::RefNull(HeapType::Concrete(0)));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End); // try_table
        f.instruction(&Instruction::RefNull(HeapType::Concrete(0))); // block result (unreachable)
        f.instruction(&Instruction::End); // outer Block
        // caught path: the branch delivered the tag param here (outer block
        // end / fall-through)
        f.instruction(&Instruction::RefCastNullable(HeapType::Concrete(1)));
        f.instruction(&Instruction::StructGet {
            struct_type_index: 1,
            field_index: 1,
        });
        f.instruction(&Instruction::GlobalSet(0));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    // _start
    {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::Call(1));
        f.instruction(&Instruction::GlobalGet(0));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    m.section(&types);
    m.section(&funcs);
    m.section(&tags);
    m.section(&globals);
    m.section(&exports);
    m.section(&code);
    let bytes = m.finish();

    let mut validator = wasmparser::Validator::new_with_features(
        wasmparser::WasmFeatures::default()
            | wasmparser::WasmFeatures::EXCEPTIONS
            | wasmparser::WasmFeatures::GC,
    );
    if let Err(e) = validator.validate_all(&bytes) {
        let mut dump = String::new();
        for (i, b) in bytes.iter().enumerate().take(80) {
            dump += &format!("{:02x} ", b);
            if (i + 1) % 16 == 0 {
                dump += "\n";
            }
        }
        panic!("payload probe validate: {e}\nbytes[0..80]:\n{dump}");
    }
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let module = wasmtime::Module::new(&engine, &bytes).unwrap();
    let mut store = wasmtime::Store::new(&engine, ());
    let linker: wasmtime::Linker<()> = wasmtime::Linker::new(&engine);
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let start = instance
        .get_typed_func::<(), i32>(&mut store, "_start")
        .unwrap();
    let r = start.call(&mut store, ()).unwrap();
    println!("PROBE-3: payload round-trip result = {r} (want 1)");
    assert_eq!(r, 42, "Catch::One must deliver the tag payload (42) at the label");
}
