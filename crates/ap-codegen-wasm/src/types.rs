//! The spec-07 type hierarchy + module-layout tables (S-07-43 emission
//! order). One source of truth for type indices, field offsets, singleton
//! global indices, and builtin-type-object layout shared by the emitter
//! (`lower.rs`) and the tests.
//!
//! Layout note (spec-07 §2 preamble): the wat shapes in spec 07 are normative
//! *layouts*; the emitter MAY choose equivalent encodings of the same fields.
//! This backend keeps the declared field ORDER (so the first-field `$type`
//! ref matches spec-07's "field 0" wording everywhere) and declares every
//! type inside ONE rec group so the whole hierarchy is mutually recursive
//! with absolute (== group-relative) type indices.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use wasm_encoder::{FieldType, RefType, StorageType, ValType};

/// The `(ref null $pyobj)` value type.
pub fn pyobj_ref() -> ValType {
    ValType::Ref(RefType {
        nullable: true,
        heap_type: wasm_encoder::HeapType::Concrete(T_PYOBJ),
    })
}

fn nullable_concrete(ty: u32) -> ValType {
    ValType::Ref(RefType {
        nullable: true,
        heap_type: wasm_encoder::HeapType::Concrete(ty),
    })
}

fn field(ty: ValType, mutable: bool) -> FieldType {
    FieldType {
        element_type: StorageType::Val(ty),
        mutable,
    }
}

fn field_i32(mutable: bool) -> FieldType {
    field(ValType::I32, mutable)
}
fn field_i64(mutable: bool) -> FieldType {
    field(ValType::I64, mutable)
}
fn field_f64(mutable: bool) -> FieldType {
    field(ValType::F64, mutable)
}
fn field_ref(ty: u32, mutable: bool) -> FieldType {
    field(nullable_concrete(ty), mutable)
}
fn field_i8(mutable: bool) -> FieldType {
    FieldType {
        element_type: StorageType::I8,
        mutable,
    }
}
fn field_i16(mutable: bool) -> FieldType {
    FieldType {
        element_type: StorageType::I16,
        mutable,
    }
}

// ---------------------------------------------------------------------------
// Type indices (spec-07 §2–§8 shapes; ONE rec group, indices absolute).
// ---------------------------------------------------------------------------

pub const T_PYOBJ: u32 = 0;
pub const T_TYPE: u32 = 1;
pub const T_INT: u32 = 2;
pub const T_BIGINT: u32 = 3;
pub const T_LIMB_ARRAY: u32 = 4;
pub const T_FLOAT: u32 = 5;
pub const T_STR: u32 = 6;
pub const T_STR16: u32 = 7;
pub const T_STR32: u32 = 8;
pub const T_BYTES: u32 = 9;
pub const T_BYTE_ARR: u32 = 10;
pub const T_I16_ARR: u32 = 11;
pub const T_I32_ARR: u32 = 12;
pub const T_LIST: u32 = 13;
pub const T_OBJ_ARR: u32 = 14;
pub const T_TUPLE: u32 = 15;
pub const T_OBJ_ARR_RO: u32 = 16;
pub const T_DICT: u32 = 17;
pub const T_DICT_ENTRY: u32 = 18;
pub const T_ENTRY_ARR: u32 = 19;
pub const T_IDX_ARR: u32 = 20;
pub const T_SET: u32 = 21;
pub const T_EXCEPTION: u32 = 22;
pub const T_TB_FRAME: u32 = 23;
pub const T_FUNCTION: u32 = 24;
pub const T_CELL: u32 = 25;
pub const T_CELL_ARR: u32 = 26;
pub const T_CODE: u32 = 27;
pub const T_SLOT_ARR: u32 = 28;
pub const T_TYPE_ARR: u32 = 29;
pub const T_METHOD: u32 = 30;
pub const T_GENERATOR: u32 = 31;
pub const T_INSTANCE: u32 = 32;
/// Types 0..=34 ride the single rec group (34 = `$descriptor`, leaf 03.03 —
/// append-only, nothing renumbered).
pub const REC_GROUP_LEN: u32 = 35;
/// `$complex` rec-group index (S-07 §9: two binary64 halves).
pub const T_COMPLEX: u32 = 33;
/// `$descriptor` rec-group index (leaf 03.03: property/staticmethod/
/// classmethod payloads, S-03-20/21).
pub const T_DESCRIPTOR: u32 = 34;
// $descriptor: type(0) kind(1) fget(2) fset(3) fdel(4)
pub const F_DESCR_KIND: u32 = 1;
pub const F_DESCR_FGET: u32 = 2;
pub const F_DESCR_FSET: u32 = 3;
pub const F_DESCR_FDEL: u32 = 4;

/// Function types (declared AFTER the rec group; ids start at 34 =
/// REC_GROUP_LEN — the rec group grew by `$complex` in leaf 03.02b).
pub const FT_FD_WRITE: u32 = 35;
pub const FT_ABORT_MSG: u32 = 36;
pub const FT_VOID: u32 = 37;
pub const FT_PYCALL: u32 = 38;
// ---------------------------------------------------------------------------
// Prelude function types (leaf 03.02b; appended, existing ids untouched).
// `o` = (ref null $pyobj), `big` = (ref null $bigint), `str` = (ref null $str).
// ---------------------------------------------------------------------------
pub const FT_U_O: u32 = 39; // (i32 unused, o) -> o          bool_obj
pub const FT_IOP_O: u32 = 57; // (i32 op, o, o) -> o         binop dispatch
pub const FT_O_O: u32 = 58; // (o) -> o                       len
pub const FT_O_STR: u32 = 40; // (o) -> $str                 py_str/int_to_str
pub const FT_O_L: u32 = 41; // (o) -> i64                    hash
pub const FT_O_I: u32 = 42; // (o) -> i32                    truthy/type_index
pub const FT_OO_I: u32 = 43; // (o, o) -> i32               eq primitives
pub const FT_TOP_I: u32 = 44; // (i32 op, o, o) -> i32       compare dispatch
pub const FT_PL_ONE: u32 = 45; // (i32 bufptr, o $pyobj) -> i32  print_one (casts to $str inside)
pub const FT_PL_FLUSH: u32 = 46; // (i32 bufptr, i32 end) -> ()
pub const FT_L_O: u32 = 47; // (i64) -> o                    int/float from i64
pub const FT_L_OI2: u32 = 48; // (i64, i32 ptr) -> i32       append digits
pub const FT_F_O: u32 = 49; // (f64) -> o                    float_from_i64 is L_O; this is float->obj (unused v1)
pub const FT_SFB: u32 = 50; // (i32 ptr, i32 len) -> str     str_from_bytes
pub const FT_BIG_NORM: u32 = 51; // (big) -> big
pub const FT_BIG_FI64: u32 = 52; // (i64) -> big
pub const FT_BIG_CMP: u32 = 53; // (big, big) -> i32
pub const FT_BIG_BITS: u32 = 54; // (big) -> i64
/// 61 FT_PY_RO: the uniform user-function calling convention (S-07-23,
/// args array immutable — see the append_prelude_fts tail comment).
pub const FT_PY_RO: u32 = 62;
pub const FT_L_L: u32 = 55; // (i64) -> i64                    hash_i64 core
pub const FT_F_L: u32 = 56; // (f64) -> i64                   float_hash

// ---------------------------------------------------------------------------
// Struct field indices (spec-07 layouts, declared order).
// ---------------------------------------------------------------------------

pub const F_TYPE: u32 = 0; // every $sub-struct's field 0: the $type ref

// $int: type(0) kind(1) smi(2) big(3)
pub const F_INT_KIND: u32 = 1;
pub const F_INT_SMI: u32 = 2;
pub const F_INT_BIG: u32 = 3;
// $float: type(0) val(1)
pub const F_FLOAT_VAL: u32 = 1;
// $str: type(0) hash(1) kind(2) len(3) data(4)
pub const F_STR_HASH: u32 = 1;
pub const F_STR_KIND: u32 = 2;
pub const F_STR_LEN: u32 = 3;
pub const F_STR_DATA: u32 = 4;
// $str16: +data16(5); $str32: +data32(5)
pub const F_STR16_DATA: u32 = 5;
pub const F_STR32_DATA: u32 = 5;
// $bytes: type(0) hash(1) len(2) data(3)
pub const F_BYTES_HASH: u32 = 1;
pub const F_BYTES_LEN: u32 = 2;
pub const F_BYTES_DATA: u32 = 3;
// $list: type(0) len(1) cap(2) items(3)
pub const F_LIST_LEN: u32 = 1;
pub const F_LIST_CAP: u32 = 2;
pub const F_LIST_ITEMS: u32 = 3;
// $tuple: type(0) items(1)
pub const F_TUPLE_ITEMS: u32 = 1;
// $dict/$set: type(0) used(1) fill(2) mask(3) perturb_shift(4) index(5) entries(6)
pub const F_DICT_USED: u32 = 1;
pub const F_DICT_FILL: u32 = 2;
pub const F_DICT_MASK: u32 = 3;
pub const F_DICT_PERTURB: u32 = 4;
pub const F_DICT_INDEX: u32 = 5;
pub const F_DICT_ENTRIES: u32 = 6;
// $dict_entry: hash(0) key(1) value(2)
pub const F_ENTRY_HASH: u32 = 0;
pub const F_ENTRY_KEY: u32 = 1;
pub const F_ENTRY_VALUE: u32 = 2;
// $type: type(0) name(1) mro(2) slots(3) bases(4) dict(5)
pub const F_TYPE_NAME: u32 = 1;
pub const F_TYPE_MRO: u32 = 2;
pub const F_TYPE_SLOTS: u32 = 3;
pub const F_TYPE_BASES: u32 = 4;
pub const F_TYPE_DICT: u32 = 5;
// $code: name(0) nparams(1) flags(2) param_names(3) func_idx(4)
pub const F_CODE_NAME: u32 = 0;
pub const F_CODE_NPARAMS: u32 = 1;
pub const F_CODE_FLAGS: u32 = 2;
pub const F_CODE_PARAM_NAMES: u32 = 3;
pub const F_CODE_FUNC_IDX: u32 = 4;
// $function: type(0) code(1) defaults(2) kwdefaults(3) cells(4) globals(5)
pub const F_FN_CODE: u32 = 1;
pub const F_FN_DEFAULTS: u32 = 2;
pub const F_FN_KWDEFAULTS: u32 = 3;
pub const F_FN_CELLS: u32 = 4;
pub const F_FN_GLOBALS: u32 = 5;
// $cell: v(0)
pub const F_CELL_V: u32 = 0;
// $exception: type(0) args(1) tb(2) context(3) cause(4) suppress(5)
pub const F_EXC_ARGS: u32 = 1;
pub const F_EXC_TB: u32 = 2;
pub const F_EXC_CONTEXT: u32 = 3;
pub const F_EXC_CAUSE: u32 = 4;
pub const F_EXC_SUPPRESS: u32 = 5;
// $tb_frame: next(0) filename(1) funcname(2) lineno(3) col(4) end_lineno(5)
pub const F_TB_NEXT: u32 = 0;
pub const F_TB_FILENAME: u32 = 1;
pub const F_TB_FUNCNAME: u32 = 2;
pub const F_TB_LINENO: u32 = 3;
pub const F_TB_COL: u32 = 4;
pub const F_TB_END_LINENO: u32 = 5;
// $instance: type(0) attrs(1)
pub const F_INSTANCE_ATTRS: u32 = 1;
// $method: type(0) self(1) fn(2)
pub const F_METHOD_SELF: u32 = 1;
pub const F_METHOD_FN: u32 = 2;
// $generator: type(0) fn(1) state(2) locals(3) value(4) exn(5)
pub const F_GEN_FN: u32 = 1;
pub const F_GEN_STATE: u32 = 2;
pub const F_GEN_LOCALS: u32 = 3;
pub const F_GEN_VALUE: u32 = 4;
pub const F_GEN_EXN: u32 = 5;
// $bigint: sign(0) limbs(1)
pub const F_BIG_SIGN: u32 = 0;
pub const F_BIG_LIMBS: u32 = 1;

// ---------------------------------------------------------------------------
// Globals (spec-07 §12.4: singletons + rt-init flag).
// ---------------------------------------------------------------------------

pub const G_NONE: u32 = 0;
pub const G_TRUE: u32 = 1;
pub const G_FALSE: u32 = 2;
pub const G_ELLIPSIS: u32 = 3;
pub const G_RT_DONE: u32 = 4;
/// Per-builtin-type $type refs (built at rt_init, order = TYPE_ORDER).
pub const G_TYPE_BASE: u32 = 5;

/// Builtin type objects rt_init builds, in globals-slot order. Names match
/// ap-obj `Value::type_name()` spellings; bool is its own $type over $int
/// payloads per S-07-8; NoneType/ellipsis name the singleton type objects.
pub const TYPE_ORDER: [&str; 16] = [
    "int",
    "float",
    "bool",
    "str",
    "bytes",
    "list",
    "tuple",
    "dict",
    "set",
    "frozenset",
    "NoneType",
    "ellipsis",
    "type",
    "object",
    "function",
    "BaseException",
];

/// Globals slot of a builtin type by name.
pub fn builtin_type_global(name: &str) -> Option<u32> {
    TYPE_ORDER
        .iter()
        .position(|n| *n == name)
        .map(|i| G_TYPE_BASE + i as u32)
}

pub const GLOBAL_COUNT: u32 = G_TYPE_BASE + TYPE_ORDER.len() as u32;

// ---------------------------------------------------------------------------
// Leaf 03.03: the builtin EXCEPTION class tree (S-03-32). rt_init builds
// these as real $type objects over the BaseException root AFTER the base
// TYPE_ORDER list. Each row: (class name, direct base name). The order is
// the emission order (=> global order); bases must precede users.
// ---------------------------------------------------------------------------

/// (name, base) rows for the exception tree rt_init builds. Names/spellings
/// match ap-run exc_hier.rs (S-03-32's observed 3.14 tree, corpus slice).
pub const EXC_TREE: [(&str, &str); 24] = [
    ("BaseException", "object"),
    ("Exception", "BaseException"),
    ("TypeError", "Exception"),
    ("ValueError", "Exception"),
    ("AttributeError", "Exception"),
    ("LookupError", "Exception"),
    ("KeyError", "LookupError"),
    ("IndexError", "LookupError"),
    ("NameError", "Exception"),
    ("UnboundLocalError", "NameError"),
    ("RuntimeError", "Exception"),
    ("ZeroDivisionError", "ArithmeticError"),
    ("ArithmeticError", "Exception"),
    ("StopIteration", "Exception"),
    ("AssertionError", "Exception"),
    ("OSError", "Exception"),
    ("MemoryError", "Exception"),
    ("EOFError", "Exception"),
    ("ImportError", "Exception"),
    ("SystemExit", "BaseException"),
    ("KeyboardInterrupt", "BaseException"),
    ("GeneratorExit", "BaseException"),
    ("Warning", "Exception"),
    ("NotImplementedError", "RuntimeError"),
];

/// Total $type globals after rt_init: TYPE_ORDER + the exception tree.
pub const TYPE_GLOBAL_COUNT: u32 = G_TYPE_BASE + TYPE_ORDER.len() as u32 + EXC_TREE.len() as u32;

/// Globals slot of a builtin EXCEPTION class by name (the exception tree
/// lives at TYPE_ORDER.len()..). Names in TYPE_ORDER win first.
pub fn exc_type_global(name: &str) -> Option<u32> {
    if let Some(i) = TYPE_ORDER.iter().position(|n| *n == name) {
        return Some(G_TYPE_BASE + i as u32);
    }
    EXC_TREE
        .iter()
        .position(|(n, _)| *n == name)
        .map(|i| G_TYPE_BASE + TYPE_ORDER.len() as u32 + i as u32)
}

/// The module source FILENAME lives in the string table; _start builds its
/// $str into G_SOURCE_FILE. The lowering writes the filename literal at this
/// StrTab index FIRST (deterministic prefix).
pub fn source_file_str_marker() -> &'static str {
    "\u{0}__ap_source_file__"
}

// ---------------------------------------------------------------------------
// Slot-id table (S-07-19, FROZEN; shared with the native dispatch enum,
// append-only, never renumbered).
// ---------------------------------------------------------------------------

pub const SLOT_BOOL: u32 = 0;
pub const SLOT_LEN: u32 = 1;
pub const SLOT_ITER: u32 = 2;
pub const SLOT_NEXT: u32 = 3;
pub const SLOT_GETITEM: u32 = 4;
pub const SLOT_SETITEM: u32 = 5;
pub const SLOT_DELITEM: u32 = 6;
pub const SLOT_CONTAINS: u32 = 7;
pub const SLOT_CALL: u32 = 8;
pub const SLOT_REPR: u32 = 9;
pub const SLOT_STR: u32 = 10;
pub const SLOT_HASH: u32 = 11;
pub const SLOT_EQ: u32 = 12;
pub const SLOT_NE: u32 = 13;
pub const SLOT_LT: u32 = 14;
pub const SLOT_LE: u32 = 15;
pub const SLOT_GT: u32 = 16;
pub const SLOT_GE: u32 = 17;
pub const SLOT_NEG: u32 = 18;
pub const SLOT_POS: u32 = 19;
pub const SLOT_ABS: u32 = 20;
pub const SLOT_INVERT: u32 = 21;
pub const SLOT_ADD: u32 = 22;
pub const SLOT_RADD: u32 = 23;
pub const SLOT_SUB: u32 = 24;
pub const SLOT_RSUB: u32 = 25;
pub const SLOT_MUL: u32 = 26;
pub const SLOT_RMUL: u32 = 27;
pub const SLOT_TRUEDIV: u32 = 28;
pub const SLOT_RTRUEDIV: u32 = 29;
pub const SLOT_FLOORDIV: u32 = 30;
pub const SLOT_RFLOORDIV: u32 = 31;
pub const SLOT_MOD: u32 = 32;
pub const SLOT_RMOD: u32 = 33;
pub const SLOT_POW: u32 = 34;
pub const SLOT_RPOW: u32 = 35;
pub const SLOT_LSHIFT: u32 = 36;
pub const SLOT_RLSHIFT: u32 = 37;
pub const SLOT_RSHIFT: u32 = 38;
pub const SLOT_RRSHIFT: u32 = 39;
pub const SLOT_AND: u32 = 40;
pub const SLOT_RAND: u32 = 41;
pub const SLOT_XOR: u32 = 42;
pub const SLOT_RXOR: u32 = 43;
pub const SLOT_OR: u32 = 44;
pub const SLOT_ROR: u32 = 45;
pub const SLOT_INT: u32 = 46;
pub const SLOT_INDEX: u32 = 47;
pub const SLOT_IADD: u32 = 48;
pub const SLOT_ISUB: u32 = 49;
pub const SLOT_IMUL: u32 = 50;
pub const SLOT_ITRUEDIV: u32 = 51;
pub const SLOT_IFLOORDIV: u32 = 52;
pub const SLOT_IMOD: u32 = 53;
pub const SLOT_IPOW: u32 = 54;
pub const SLOT_IAND: u32 = 55;
pub const SLOT_IOR: u32 = 56;
pub const SLOT_IXOR: u32 = 57;
pub const SLOT_ILSHIFT: u32 = 58;
pub const SLOT_IRSHIFT: u32 = 59;
pub const SLOT_ENTER: u32 = 60;
pub const SLOT_EXIT: u32 = 61;
pub const SLOT_GET: u32 = 62;
pub const SLOT_SET: u32 = 63;
pub const SLOT_DELETE: u32 = 64;
pub const SLOT_SETATTR: u32 = 65;
pub const SLOT_GETATTR: u32 = 66;
pub const SLOT_GETATTRIBUTE: u32 = 67;
pub const SLOT_DELATTR: u32 = 68;
pub const SLOT_DEL: u32 = 69;
pub const SLOT_AWAIT: u32 = 70;
pub const SLOT_AITER: u32 = 71;
pub const SLOT_ANEXT: u32 = 72;
pub const SLOT_FORMAT: u32 = 73;
pub const SLOT_FSPATH: u32 = 74;
pub const SLOT_LENGTH_HINT: u32 = 75;

/// Slot-table size this leaf emits (ids 0..=75 per S-07-19).
pub const SLOT_COUNT: u32 = 76;

// ---------------------------------------------------------------------------
// Type-section builder (S-07-43 step 1).
// ---------------------------------------------------------------------------

/// Build the full type section: ONE rec group with the 33-type GC hierarchy,
/// then the 4 function types.
pub fn build_type_section() -> wasm_encoder::TypeSection {
    build_type_section_with_prefix(usize::MAX)
}

/// Build the type section with only the first `prefix` rec-group entries
/// (debug bisection; `usize::MAX` = the full group).
pub fn build_type_section_with_prefix(prefix: usize) -> wasm_encoder::TypeSection {
    use wasm_encoder::{CompositeInnerType, CompositeType, StructType, SubType};

    let c = |idx: u32| -> wasm_encoder::HeapType { wasm_encoder::HeapType::Concrete(idx) };
    let fr = |idx: u32, mutable: bool| -> FieldType { field(nullable_concrete(idx), mutable) };
    let array_ty = |elem: FieldType| -> SubType {
        SubType {
            is_final: true,
            supertype_idxs: vec![],
            composite_type: CompositeType {
                inner: CompositeInnerType::Array(wasm_encoder::ArrayType(elem)),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };
    // Bare struct (not a $pyobj subtype). `is_final` = !subtyped_by:
    // WasmGC requires a supertype to be declared OPEN (non-final) for any
    // `sub` declaration to reference it.
    let bare = |fields: Vec<FieldType>| -> SubType {
        SubType {
            is_final: true,
            supertype_idxs: vec![],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: fields.into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };
    // The $pyobj ROOT: non-final (everything subtypes it), no supertype.
    let pyobj_root = |fields: Vec<FieldType>| -> SubType {
        SubType {
            is_final: false,
            supertype_idxs: vec![],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: fields.into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };
    // Struct sub $pyobj: field 0 is the $type ref, rest follow.
    let sub = |fields: Vec<FieldType>| -> SubType {
        let mut all = vec![field_ref(T_TYPE, false)];
        all.extend(fields);
        SubType {
            is_final: true,
            supertype_idxs: vec![T_PYOBJ],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: all.into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };
    // $str16/$str32 sub $str (S-07-10/G-4: declared now, emitted later).
    // Field 0 stays (ref null $type) — the DYNAMIC type pointer per
    // spec-07 §2; only the payload fields differ from $str.
    let sub_str = |fields: Vec<FieldType>| -> SubType {
        let mut all = vec![field_ref(T_TYPE, false)];
        all.extend(fields);
        SubType {
            is_final: false,
            supertype_idxs: vec![T_STR],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: all.into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };
    // OPEN (non-final) sub of $pyobj — for types that are themselves
    // subtyped later ($str: $str16/$str32 subtype it).
    let sub_open = |fields: Vec<FieldType>| -> SubType {
        let mut all = vec![field_ref(T_TYPE, false)];
        all.extend(fields);
        SubType {
            is_final: false,
            supertype_idxs: vec![T_PYOBJ],
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: all.into_boxed_slice(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        }
    };

    let dict_fields = || -> Vec<FieldType> {
        vec![
            field_i32(true),        // used (dict_set grows, leaf 03.03)
            field_i32(true),        // fill
            field_i32(false),       // mask
            field_i32(false),       // perturb_shift
            fr(T_IDX_ARR, false),   // index
            fr(T_ENTRY_ARR, true),  // entries (rebuild on grow)
        ]
    };

    let group: Vec<SubType> = vec![
        // 0 $pyobj — OPEN root (everything subtypes it), one $type field.
        pyobj_root(vec![fr(T_TYPE, false)]),
        // 1 $type sub $pyobj
        sub(vec![
            fr(T_STR, false),      // name
            fr(T_TYPE_ARR, false), // mro
            fr(T_SLOT_ARR, false), // slots
            fr(T_OBJ_ARR, false),  // bases
            fr(T_DICT, false),     // dict
        ]),
        // 2 $int sub $pyobj
        sub(vec![
            field_i32(false),    // kind (0 smi / 1 big)
            field_i64(false),    // smi (i61)
            fr(T_BIGINT, false), // big
        ]),
        // 3 $bigint — internal payload, NOT a $pyobj (S-07-6)
        bare(vec![field_i32(false), fr(T_LIMB_ARRAY, false)]),
        // 4 $limb_array
        array_ty(field_i32(true)),
        // 5 $float sub $pyobj
        sub(vec![field_f64(false)]),
        // 6 $str — OPEN sub of $pyobj ($str16/$str32 subtype it, G-4).
        // sup = $pyobj; non-final so the subtype edge from 7/8 is legal.
        sub_open(vec![
            field_i64(false),      // hash (0 = unhashed, S-07-12)
            field_i32(false),      // kind
            field_i32(false),      // len (code points)
            fr(T_BYTE_ARR, false), // data (kind 0)
        ]),
        // 7 $str16 sub $str (G-4: declared now, emitted later)
        sub_str(vec![
            field_i64(false),
            field_i32(false),
            field_i32(false),
            fr(T_BYTE_ARR, false),
            fr(T_I16_ARR, false),
        ]),
        // 8 $str32 sub $str
        sub_str(vec![
            field_i64(false),
            field_i32(false),
            field_i32(false),
            fr(T_BYTE_ARR, false),
            fr(T_I32_ARR, false),
        ]),
        // 9 $bytes sub $pyobj
        sub(vec![
            field_i64(false), // hash
            field_i32(false), // len (BYTES)
            fr(T_BYTE_ARR, false),
        ]),
        // 10 $byte_arr — elements MUTABLE (print_one/array.copy writes)
        array_ty(field_i8(true)),
        // 11 $i16_arr
        array_ty(field_i16(false)),
        // 12 $i32_arr
        array_ty(field(field_i32_val(), false)),
        // 13 $list sub $pyobj — items are the RO array type: literals are
        // built with array.new_fixed (WasmGC requires an immutable array
        // type there); v1 lists on this slice never mutate in place, and
        // mutation (03.02c) rebuilds via array.new_fixed as well.
        sub(vec![
            field_i32(false), // len
            field_i32(false), // cap
            fr(T_OBJ_ARR_RO, false), // items (immutable: literals via
                              // array.new_fixed; mutation rebuilds)
        ]),
        // 14 $obj_arr — mut pyobj array
        array_ty(field(pyobj_ref(), true)),
        // 15 $tuple sub $pyobj
        sub(vec![fr(T_OBJ_ARR_RO, false)]),
        // 16 $obj_arr_ro — immutable pyobj array (array.new_fixed target)
        array_ty(field(pyobj_ref(), false)),
        // 17 $dict sub $pyobj
        sub(dict_fields()),
        // 18 $dict_entry — bare struct (key/value MUTABLE: dict_set
        // overwrites the value in place, S-01-21 insertion order; leaf 03.03)
        bare(vec![
            field_i64(false),           // hash
            field_pyobj_ref(),          // key (immutable: identity)
            field(pyobj_ref(), true),   // value (mutable)
        ]),
        // 19 $entry_arr — mut array of $dict_entry
        array_ty(field(
            ValType::Ref(RefType {
                nullable: true,
                heap_type: c(T_DICT_ENTRY),
            }),
            true,
        )),
        // 20 $idx_arr
        array_ty(field_i32(true)),
        // 21 $set sub $pyobj — same probing fields, no values (S-07-16)
        sub(dict_fields()),
        // 22 $exception sub $pyobj
        sub(vec![
            fr(T_TUPLE, false),     // args
            fr(T_TB_FRAME, true),   // tb
            fr(T_EXCEPTION, false), // context
            fr(T_EXCEPTION, false), // cause
            field_i32(false),       // suppress_context
        ]),
        // 23 $tb_frame — bare struct (lineno MUTABLE: Stmt::Line pokes the
        // current frame's line, leaf 03.03; filename filled at _start)
        bare(vec![
            fr(T_TB_FRAME, true), // next
            fr(T_STR, true),      // filename (lowered fill at _start)
            fr(T_STR, false),     // funcname
            field_i32(true),      // lineno
            field_i32(false),     // col
            field_i32(false),     // end_lineno
        ]),
        // 24 $function sub $pyobj (S-07 §4)
        sub(vec![
            fr(T_CODE, false),     // code
            fr(T_OBJ_ARR, false),  // defaults (def-time)
            fr(T_DICT, false),     // kwdefaults
            fr(T_CELL_ARR, false), // cells
            fr(T_DICT, false),     // globals
        ]),
        // 25 $cell — bare struct
        bare(vec![field_pyobj_ref_mut()]),
        // 26 $cell_arr — immut array of $cell refs
        array_ty(field(
            ValType::Ref(RefType {
                nullable: true,
                heap_type: c(T_CELL),
            }),
            false,
        )),
        // 27 $code — bare module-data struct
        bare(vec![
            fr(T_STR, false),     // name
            field_i32(false),     // nparams
            field_i32(false),     // flags
            fr(T_OBJ_ARR, false), // param_names
            field_i32(false),     // func_idx
        ]),
        // 28 $slot_arr — mut array of $function refs
        array_ty(field(
            ValType::Ref(RefType {
                nullable: true,
                heap_type: c(T_FUNCTION),
            }),
            true,
        )),
        // 29 $type_arr — mut array of $type refs
        array_ty(field(
            ValType::Ref(RefType {
                nullable: true,
                heap_type: c(T_TYPE),
            }),
            true,
        )),
        // 30 $method sub $pyobj
        sub(vec![
            field_pyobj_ref(), // self (never null once built)
            fr(T_FUNCTION, false),
        ]),
        // 31 $generator sub $pyobj (S-07 §8)
        sub(vec![
            fr(T_FUNCTION, false),  // fn
            field_i32(false),       // state
            fr(T_OBJ_ARR, false),   // locals
            field_pyobj_ref_mut(),  // value
            fr(T_EXCEPTION, false), // exn
        ]),
        // 32 $instance sub $pyobj (S-07-17)
        sub(vec![fr(T_DICT, true)]), // attrs (lazy, null until first set)
        // 33 $complex sub $pyobj (S-07 §9, leaf 03.02b): two binary64 halves.
        sub(vec![field_f64(false), field_f64(false)]),
        // 34 $descriptor sub $pyobj (leaf 03.03): property/staticmethod/
        // classmethod descriptor objects (S-03-20/21). kind: 0=property,
        // 1=staticmethod, 2=classmethod; fget/fset/fdel carry the callables
        // (null = absent, per property's read-only shape).
        sub(vec![
            field_i32(false),
            field_pyobj_ref(), // fget
            field_pyobj_ref(), // fset
            field_pyobj_ref(), // fdel
        ]),
    ];

    let mut types = wasm_encoder::TypeSection::new();
    let group: Vec<SubType> = if prefix >= group.len() {
        group
    } else {
        group.into_iter().take(prefix).collect()
    };
    types.ty().rec(group);

    // Function types after the rec group.
    types.ty().function(
        [ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        [ValType::I32],
    ); // FT_FD_WRITE
       // FT_ABORT_MSG: (param anyref). Spec-07-41 declares (param (ref $str));
       // v1 widens the DECLARED import param to anyref because wasmtime's
       // host-func matching requires the host-side concrete $str struct type
       // (a self-referential StructType the host API cannot express yet).
       // Call sites pass (ref $str) values — the widening is an ordinary
       // subsumption. DEVIATION recorded for 03.05 (import-surface owner).
    types.ty().function(
        [ValType::Ref(RefType {
            nullable: true,
            heap_type: wasm_encoder::HeapType::Abstract {
                shared: false,
                ty: wasm_encoder::AbstractHeapType::Any,
            },
        })],
        [],
    );
    types.ty().function([], []); // FT_VOID
    types.ty().function(
        [
            nullable_concrete(T_FUNCTION),
            fr(T_OBJ_ARR, false).element_type.unpack(),
            nullable_concrete(T_DICT),
        ],
        [nullable_concrete(T_PYOBJ)],
    ); // FT_PYCALL — (fn, args, kwargs) -> pyobj
    append_prelude_fts(&mut types);

    types
}

fn field_i32_val() -> ValType {
    ValType::I32
}
fn field_pyobj_ref() -> FieldType {
    field(pyobj_ref(), false)
}
fn field_pyobj_ref_mut() -> FieldType {
    field(pyobj_ref(), true)
}

// ---------------------------------------------------------------------------
// Leaf 03.02b: complex type global + prelude FT builders (S-07 §9 v1).
// ---------------------------------------------------------------------------

/// Complex type GLOBAL slot (declared AFTER the builtin $type globals;
/// rt_init leaves it null — bool(v) never inspects it on the corpus slice).
pub const G_COMPLEX: u32 = GLOBAL_COUNT;
/// Total globals emitted: singletons + rt_done + 16 builtin types + complex.
pub const GLOBAL_COUNT_TOTAL: u32 = GLOBAL_COUNT + 1;

/// First MODULE-NAMESPACE global slot (leaf 03.02c): every module-level
/// Python name gets one `(ref null $pyobj)` global here, in first-assignment
/// order. Function bodies read/write these (LEGB: locals shadow by living in
/// wasm locals), and unbound reads test the rt_init null before the error
/// path (fail-loud, never null propagation — design §1).
pub const G_MOD_BASE: u32 = GLOBAL_COUNT_TOTAL;

// ---------------------------------------------------------------------------
// Leaf 03.03 globals: class frame/exception machinery (appended after the
// module namespace; counts are add-ons so existing indices never move).
// ---------------------------------------------------------------------------

/// Frame-stack head (null = empty). Frames are $tb_frame structs linked via
/// $next; the CURRENT frame is the head, its $funcname names the fn.
pub const G_FRAME_HEAD: u32 = G_MOD_BASE;
/// The frame STACK DEPTH (i32) — handler cut points compare against it.
pub const G_FRAME_DEPTH: u32 = G_MOD_BASE + 1;
/// Current line of the CURRENT frame (Stmt::Line stores; throw consumes).
pub const G_CUR_LINE: u32 = G_MOD_BASE + 2;
/// The module's source FILENAME $str (built by _start from the data segment;
/// traceback frames + host boundary render use it).
pub const G_SOURCE_FILE: u32 = G_MOD_BASE + 3;
/// The in-flight exception being handled by the INNERMOST active handler
/// (bare `raise` re-raises this; S-02-28). Null = no active except.
pub const G_ACTIVE_EXC: u32 = G_MOD_BASE + 4;
/// Count of globals this leaf adds after the module namespace.
pub const G_LEAF03_COUNT: u32 = 5;

/// Append the prelude FT declarations (FT_IOP_O..FT_L_L) to the type
/// section — called by `build_type_section` after FT_PYCALL.
pub fn append_prelude_fts(types: &mut wasm_encoder::TypeSection) {
    let o = || pyobj_ref();
    let big = || nullable_concrete(T_BIGINT);
    let sg = || nullable_concrete(T_STR);
    let i32v = || ValType::I32;
    let i64v = || ValType::I64;
    let f64v = || ValType::F64;
    // 38 FT_U_O: (unused-op, o) -> o
    types.ty().function([i32v(), o()], [o()]);
    // 39 FT_O_STR: (o) -> $str
    types.ty().function([o()], [sg()]);
    // 40 FT_O_L: (o) -> i64
    types.ty().function([o()], [i64v()]);
    // 41 FT_O_I: (o) -> i32
    types.ty().function([o()], [i32v()]);
    // 42 FT_OO_I: (o, o) -> i32
    types.ty().function([o(), o()], [i32v()]);
    // 43 FT_TOP_I: (op, o, o) -> i32
    types.ty().function([i32v(), o(), o()], [i32v()]);
    // 44 FT_PL_ONE: (bufptr, o) -> new-bufptr. Param is $pyobj: _start stashes
    // py_str's return in an oref local (declared $pyobj); print_one casts to
    // $str internally (RefCastNullable before every struct.get).
    types.ty().function([i32v(), o()], [i32v()]);
    // 45 FT_PL_FLUSH: (bufptr, end) -> ()
    types.ty().function([i32v(), i32v()], []);
    // 46 FT_L_O: (i64) -> o
    types.ty().function([i64v()], [o()]);
    // 47 FT_L_OI2: (i64, ptr) -> new-ptr
    types.ty().function([i64v(), i32v()], [i32v()]);
    // 48 FT_F_O: (f64) -> o (declared; used by later leaves)
    types.ty().function([f64v()], [o()]);
    // 49 FT_SFB: (ptr, len) -> $str
    types.ty().function([i32v(), i32v()], [sg()]);
    // 50 FT_BIG_NORM: (big) -> big
    types.ty().function([big()], [big()]);
    // 51 FT_BIG_FI64: (i64) -> big
    types.ty().function([i64v()], [big()]);
    // 52 FT_BIG_CMP: (big, big) -> i32
    types.ty().function([big(), big()], [i32v()]);
    // 53 FT_BIG_BITS: (big) -> i64
    types.ty().function([big()], [i64v()]);
    // 54 FT_L_L: (i64) -> i64
    types.ty().function([i64v()], [i64v()]);
    // 55 FT_F_L: (f64) -> i64
    types.ty().function([f64v()], [i64v()]);
    // 56 FT_IOP_O: (op, o, o) -> o
    types.ty().function([i32v(), o(), o()], [o()]);
    // 57 FT_O_O: (o) -> o                        len / passthrough
    types.ty().function([o()], [o()]);
    // 58 FT_BB_B: (big, big) -> big              bigint limb arithmetic
    types.ty().function([big(), big()], [big()]);
    // 59 FT_BI_B: (big, i64) -> big              shift by immediate
    types.ty().function([big(), i64v()], [big()]);
    // 60 FT_BI_L: (big, i64) -> i64              mod-smi remainder
    types.ty().function([big(), i64v()], [i64v()]);
    // 61 FT_PY_RO (leaf 03.02c): the uniform user-function calling
    // convention (S-07-23). Args ride the RO array type because call sites
    // build them with array.new_fixed (WasmGC requires an immutable array
    // type there); callees only READ args (params bind to locals), so the
    // mutability the spec-07 `(ref $obj_arr)` spelling implies is never
    // exercised. DEVIATION from S-07-23's `(ref $obj_arr)` recorded here:
    // same layout, immutable — swap is a one-line FT change if a later
    // leaf needs a mutable args array (RO->mut copy at the boundary).
    types.ty().function(
        [
            nullable_concrete(T_FUNCTION),
            nullable_concrete(T_OBJ_ARR_RO),
            nullable_concrete(T_DICT),
        ],
        [o()],
    );
    append_class_exc_fts(types);
}

// ---------------------------------------------------------------------------
// Leaf 03.03 FTs: classes + exceptions (S-07-17/18/19/28/29/30). Appended
// AFTER FT_PY_RO (63..); existing ids untouched (shifted once for the
// $descriptor rec growth, pre-commit). `o` = (ref null $pyobj),
// `str` = (ref null $str), `ty` = (ref null $type), `dict` = (ref null $dict),
// `exc` = (ref null $exception), `roarr` = (ref null $obj_arr_ro),
// `typarr` = (ref null $type_arr), `tb` = (ref null $tb_frame).
// ---------------------------------------------------------------------------

pub const FT_TAG: u32 = 63; // (exc) -> ()        the $tag_pyexc payload type
pub const FT_ATTR_GET: u32 = 64; // (o, str) -> o         S-03-9/12 lookup
pub const FT_ATTR_SET: u32 = 65; // (o, str, o) -> ()     S-03-10
pub const FT_ATTR_DEL: u32 = 66; // (o, str) -> ()        S-03-11
pub const FT_CLS_NEW: u32 = 67; // (str, typarr-basis...) -> type
pub const FT_CLS_CALL: u32 = 68; // (ty, roarr) -> o      instantiation S-03-05/06
pub const FT_ISINST: u32 = 69; // (o, ty) -> i32          S-03-31
pub const FT_ISSUB: u32 = 70; // (ty, ty) -> i32          S-03-31
pub const FT_EXC_RAISE: u32 = 71; // (o) -> o           fill tb + throw
pub const FT_EXC_MATCH: u32 = 72; // (o, o) -> i32      except-target match
pub const FT_EXC_MSG: u32 = 73; // (o) -> str           S-02-29 message rules
pub const FT_EXC_REPR: u32 = 74; // (o) -> str           repr(e) rules
pub const FT_DICT_GET: u32 = 75; // (dict, str) -> o     linear name lookup
pub const FT_DICT_SET: u32 = 76; // (dict, str, o) -> () insert/overwrite
pub const FT_DICT_HAS: u32 = 77; // (dict, str) -> i32   membership
pub const FT_FRAME_PUSH: u32 = 78; // (str fn) -> ()     prologue frame push
pub const FT_FRAME_POP: u32 = 79; // () -> ()            epilogue pop
pub const FT_FRAME_LINE: u32 = 80; // (i32 line) -> ()   Stmt::Line update
pub const FT_FRAME_CUT: u32 = 81; // () -> ()            handler truncation
pub const FT_STR_CAT: u32 = 82; // (str, str) -> str    message building
pub const FT_TYPE_NAME: u32 = 83; // (o) -> str         type(x).__name__
pub const FT_EXC_STR_OF: u32 = 84; // (o) -> str         str(x) dispatch w/ classes
pub const FT_BOOL_OF: u32 = 85; // (o) -> i32           truthy through classes
pub const FT_CALL_OF: u32 = 86; // (o, roarr) -> o      generic callable dispatch
pub const FT_LEN_OF: u32 = 87; // (o) -> o              len via slot + TypeError
pub const FT_GLOBAL_GET: u32 = 88; // (i32 slot) -> o    module-namespace read w/ NameError
pub const FT_GLOBAL_SET_EXC: u32 = 89; // (i32 slot, o) -> () reserved
pub const FT_EXC_CTX_SET: u32 = 90; // (o, o, o) -> o    context/cause chaining
pub const FT_GETATTR_SLOT: u32 = 91; // (o, str) -> o    __getattr__ hook path

pub fn append_class_exc_fts(types: &mut wasm_encoder::TypeSection) {
    let o = || pyobj_ref();
    let sg = || nullable_concrete(T_STR);
    let ty = || nullable_concrete(T_TYPE);
    let dc = || nullable_concrete(T_DICT);
    let ex = || nullable_concrete(T_EXCEPTION);
    let ro = || nullable_concrete(T_OBJ_ARR_RO);
    let ta = || nullable_concrete(T_TYPE_ARR);
    let i32v = || ValType::I32;
    // 63 FT_TAG: (exc) -> ()
    types.ty().function([ex()], []);
    // 63 FT_ATTR_GET: (o, str) -> o
    types.ty().function([o(), sg()], [o()]);
    // 64 FT_ATTR_SET: (o, str, o) -> ()
    types.ty().function([o(), sg(), o()], []);
    // 65 FT_ATTR_DEL: (o, str) -> ()
    types.ty().function([o(), sg()], []);
    // 66 FT_CLS_NEW: (str name, typarr bases-RO, roarr base-types-as-obj,
    //                 dict ns) -> ty — bases arrive as a $type_arr (pre-built
    //                 by the lowering from evaluated base objects).
    types.ty().function([sg(), ta(), ro(), dc()], [ty()]);
    // 67 FT_CLS_CALL: (ty, roarr args) -> o
    types.ty().function([ty(), ro()], [o()]);
    // 68 FT_ISINST: (o, ty) -> i32
    types.ty().function([o(), ty()], [i32v()]);
    // 69 FT_ISSUB: (ty, ty) -> i32
    types.ty().function([ty(), ty()], [i32v()]);
    // 70 FT_EXC_RAISE: (o) -> o — raises through the tag; return type keeps
    // call sites type-correct on the unreachable fall-through.
    types.ty().function([o()], [o()]);
    // 71 FT_EXC_MATCH: (o exc, o target) -> i32
    types.ty().function([o(), o()], [i32v()]);
    // 72 FT_EXC_MSG: (o) -> str
    types.ty().function([o()], [sg()]);
    // 73 FT_EXC_REPR: (o) -> str
    types.ty().function([o()], [sg()]);
    // 74 FT_DICT_GET: (dict, str) -> o
    types.ty().function([dc(), sg()], [o()]);
    // 75 FT_DICT_SET: (dict, str, o) -> ()
    types.ty().function([dc(), sg(), o()], []);
    // 76 FT_DICT_HAS: (dict, str) -> i32
    types.ty().function([dc(), sg()], [i32v()]);
    // 77 FT_FRAME_PUSH: (str fn) -> ()
    types.ty().function([sg()], []);
    // 78 FT_FRAME_POP: () -> ()
    types.ty().function([], []);
    // 79 FT_FRAME_LINE: (i32) -> ()
    types.ty().function([i32v()], []);
    // 80 FT_FRAME_CUT: () -> ()
    types.ty().function([], []);
    // 81 FT_STR_CAT: (str, str) -> str
    types.ty().function([sg(), sg()], [sg()]);
    // 82 FT_TYPE_NAME: (o) -> str
    types.ty().function([o()], [sg()]);
    // 83 FT_EXC_STR_OF: (o) -> str — str(x) with class __str__ dispatch
    types.ty().function([o()], [sg()]);
    // 84 FT_BOOL_OF: (o) -> i32 — truthy with __bool__/__len__ dispatch
    types.ty().function([o()], [i32v()]);
    // 85 FT_CALL_OF: (o callable, roarr args) -> o — user-callable dispatch
    types.ty().function([o(), ro()], [o()]);
    // 86 FT_LEN_OF: (o) -> o
    types.ty().function([o()], [o()]);
    // 87 FT_GLOBAL_GET: (i32 slot) -> o — unbound-name NameError read
    types.ty().function([i32v()], [o()]);
    // 88 FT_GLOBAL_SET_EXC: reserved (name rebind error surface, 03.05)
    types.ty().function([i32v(), o()], []);
    // 89 FT_EXC_CTX_SET: (o new_exc, o ctx, o cause) -> o — chaining; cause
    //    null = implicit-context mode (suppress=0), Some = `from` (suppress=1)
    types.ty().function([o(), o(), o()], [o()]);
    // 90 FT_GETATTR_SLOT: reserved (user __getattr__ objects, v1 dict walk)
    types.ty().function([o(), sg()], [o()]);
}

/// The `$tag_pyexc` wasm tag index (declared once, payload FT_TAG).
pub const TAG_PYEXC: u32 = 0;
