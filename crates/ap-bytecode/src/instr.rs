//! The flat instruction set (leaf spec 01 start set).
//!
//! Design: one enum, `Copy` where the payload allows. Payloads are
//! *indices* (const-pool idx, name idx, slot, code offset, site id), never
//! owned values — the tables live on the [`crate::CodeObject`]. Jump
//! targets are byte-independent instruction indices into the same
//! `Vec<Instr>` (resolved by the lowerer after patching; the interpreter
//! indexes `code.instrs[target]` directly).
//!
//! Complete-enum discipline (master.md): ForNext/Jump/etc. exist here so
//! leaf 01-core only ADDS lowering, not variants — but this leaf's
//! lowerer never emits them (functions containing those shapes fall back
//! wholesale).

/// Binary operator tag carried by the BinOp* family. Mirrors
/// `ap_ir::BinOp` (kept as a Copy local so `Instr` stays dependency-free
/// of the IR enum's Eq/Hash shape; conversion is a total `from`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinKind {
    Add,
    Sub,
    Mul,
    MatMul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitOr,
    BitXor,
    BitAnd,
}

impl BinKind {
    /// Total mapping from the IR tag.
    pub fn from_ir(op: ap_ir::BinOp) -> Self {
        match op {
            ap_ir::BinOp::Add => Self::Add,
            ap_ir::BinOp::Sub => Self::Sub,
            ap_ir::BinOp::Mul => Self::Mul,
            ap_ir::BinOp::MatMul => Self::MatMul,
            ap_ir::BinOp::Div => Self::Div,
            ap_ir::BinOp::FloorDiv => Self::FloorDiv,
            ap_ir::BinOp::Mod => Self::Mod,
            ap_ir::BinOp::Pow => Self::Pow,
            ap_ir::BinOp::LShift => Self::LShift,
            ap_ir::BinOp::RShift => Self::RShift,
            ap_ir::BinOp::BitOr => Self::BitOr,
            ap_ir::BinOp::BitXor => Self::BitXor,
            ap_ir::BinOp::BitAnd => Self::BitAnd,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::MatMul => "@",
            Self::Div => "/",
            Self::FloorDiv => "//",
            Self::Mod => "%",
            Self::Pow => "**",
            Self::LShift => "<<",
            Self::RShift => ">>",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::BitAnd => "&",
        }
    }
}

/// Unary operator tag (mirrors `ap_ir::UnaryOp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryKind {
    Plus,
    Minus,
    Invert,
    Not,
}

impl UnaryKind {
    pub fn from_ir(op: ap_ir::UnaryOp) -> Self {
        match op {
            ap_ir::UnaryOp::Plus => Self::Plus,
            ap_ir::UnaryOp::Minus => Self::Minus,
            ap_ir::UnaryOp::Invert => Self::Invert,
            ap_ir::UnaryOp::Not => Self::Not,
        }
    }
}

/// Comparison operator tag (mirrors `ap_ir::CmpOp`; chains are lowered to
/// pairwise Compare + JumpIfFalse by 01-core — the start set carries the
/// pairwise form only, chains fall back this leaf).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    Is,
    IsNot,
    In,
    NotIn,
}

impl CompareOp {
    pub fn from_ir(op: ap_ir::CmpOp) -> Self {
        match op {
            ap_ir::CmpOp::Eq => Self::Eq,
            ap_ir::CmpOp::NotEq => Self::NotEq,
            ap_ir::CmpOp::Lt => Self::Lt,
            ap_ir::CmpOp::LtE => Self::LtE,
            ap_ir::CmpOp::Gt => Self::Gt,
            ap_ir::CmpOp::GtE => Self::GtE,
            ap_ir::CmpOp::Is => Self::Is,
            ap_ir::CmpOp::IsNot => Self::IsNot,
            ap_ir::CmpOp::In => Self::In,
            ap_ir::CmpOp::NotIn => Self::NotIn,
        }
    }
}

/// One flat instruction. `Copy` — all payloads are machine words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Instr {
    /// Push `consts[idx]`.
    LoadConst(u32),
    /// Push local slot `slot` (fast path — no dict walk).
    LoadFast(u16),
    /// Push `names[idx]` from globals/builtins (Load/LoadGlobal share the
    /// name table; resolution order LEGB-minus-closure is the
    /// interpreter's job).
    LoadGlobal(u32),
    /// Store TOS into local slot `slot`; pops.
    StoreFast(u16),
    /// Store TOS into `names[idx]` (module-level `x = ...`).
    StoreGlobal(u32),
    /// Pop and discard TOS.
    PopTop,
    /// `b op a` — both operands known-smi at lowering time is NOT claimed
    /// here; the interpreter picks the int fast path per-op and falls back
    /// to the generic op on non-smi operands. `op` = BinKind.
    BinOp(BinKind),
    /// Unary op on TOS (`op a`).
    UnaryOp(UnaryKind),
    /// `b <op> a` -> push bool. Pairwise (chain lowering is 01-core).
    Compare(CompareOp),
    /// Unconditional jump to instruction index.
    Jump(u32),
    /// Pop TOS; jump if falsy.
    JumpIfFalse(u32),
    /// Pop TOS; jump if truthy.
    JumpIfTrue(u32),
    /// Peek TOS (leave it); jump if falsy. (BoolOp lowering shape, 01-core.)
    JumpIfFalseOrPop(u32),
    /// Peek TOS; jump if truthy. (BoolOp lowering shape, 01-core.)
    JumpIfTrueOrPop(u32),
    /// `iter(TOS)` — replaces TOS with the iterator.
    GetIter,
    /// `next(TOS_iterator)` -> push value; on exhaustion jump to `target`
    /// (the loop-exit point). Enum-complete for 01-core; the lowerer
    /// NEVER emits it this leaf (For-containing fns fall back).
    ForNext(u32),
    /// Call TOS-callee with `argc` positional args pushed below it
    /// (callee on top). `argc` counts only positionals — kwargs/*args
    /// shapes fall back this leaf (call-site shapes expand in 01-core 02).
    /// Carries the call's inline-cache site id.
    CallFunction { argc: u16, site: u32 },
    /// `TOS1.attr(TOS0)` -> push. `idx` names the attr; `site` is the
    /// LOAD_ATTR inline-cache site id.
    LoadAttr { idx: u32, site: u32 },
    /// `obj[key]` -> push (obj under key on stack).
    Subscript,
    /// `TOS1[TOS0] = v` where v is under both; pops all three.
    StoreSubscript,
    /// Push the `None` singleton.
    LoadConstNone,
    /// Push `True`.
    LoadConstTrue,
    /// Push `False`.
    LoadConstFalse,
    /// Push the empty tuple `()`.
    LoadConstEmptyTuple,
    /// Build a tuple from the `count` items below TOS; replaces them.
    BuildTuple(u16),
    /// Build a list from the `count` items below TOS; replaces them.
    BuildList(u16),
    /// Build a dict from `count` key/value pairs (2*count items below TOS,
    /// key first); replaces them.
    BuildDict(u16),
    /// Build a set from the `count` items below TOS; replaces them.
    BuildSet(u16),
    /// `return TOS` from the frame; pops.
    ReturnValue,
    /// `return None` (bare `return` / fall-off-the-end).
    ReturnNone,
    /// Augmented-assign VALUE op: pops `b` (rhs) then `a` (current target
    /// value), pushes `a iop b` — the IN-PLACE protocol (__iX__ first,
    /// falling back to the full binary machinery), NOT plain BinOp. This is
    /// why AugAssign bodies previously refused bytecode (list `+=` aliasing
    /// mutation; leaf 01-core/01 lifts the refusal with a dedicated opcode).
    /// `sym` is the operator's source spelling (error-message fidelity).
    AugOp { kind: BinKind, sym: &'static str },
    /// FUSED `TOS.method(args...)` — pops the receiver, pops `argc` args,
    /// calls the family-gated method directly (NO BoundMethod mint: the
    /// `out.append(x)` build-loop shape's per-iteration mint alloc was the
    /// dominant cost). `idx` names the method; semantics are methods::
    /// call_method's exact arms for the covered receivers — any uncovered
    /// shape raises SystemError-equivalent... no: the LOWERER emits this
    /// only for the exact Expr::Attribute-callee + Call shape, and the
    /// DISPATCHER falls back to mint+call_value for any receiver the fast
    /// arms don't cover (identical semantics, slower route).
    LoadAttrCall { idx: u32, argc: u16 },
    /// `obj.attr = v` — value under obj on the stack (value pushed first by
    /// the Assign arm, obj second): TOS=obj, TOS-1=value. Pops both. Routed
    /// to classes::attr_set_protocol — the tree-walker's exact store arm
    /// (user __setattr__ / descriptor / cell semantics single-sourced).
    /// `idx` names the attribute in the shared name table.
    StoreAttr { idx: u32 },
    /// Call TOS-callee with `argc` positional args followed by `n_kw`
    /// keyword-arg VALUES (in `kw_first..kw_first+n_kw` name-table order)
    /// pushed below it (callee on top). The interpreter zips values with
    /// `code.names[kw_first..]` and routes through call_value's kwargs
    /// slice — the identical binding machinery the tree-walker uses
    /// (defaults/kw-only/**kwargs binding live in the call boundary).
    /// *args/**kwargs CALL-SITE shapes (`f(*xs)` / `f(**ks)`) still fall
    /// back this leaf (runtime unpack opcodes unowned here).
    CallFunctionKw {
        argc: u16,
        kw_first: u32,
        n_kw: u16,
        site: u32,
    },
    /// Pseudo-instruction: this function's body is a tree-walk job
    /// (unsupported shape at lowering time). Never executed by the
    /// dispatch loop — entry (leaf 02) routes on it.
    FallbackToTreeWalk,
}

impl Instr {
    /// True for the jump family (targets are instruction indices the
    /// lowerer patches).
    pub fn is_jump(self) -> bool {
        matches!(
            self,
            Self::Jump(_)
                | Self::JumpIfFalse(_)
                | Self::JumpIfTrue(_)
                | Self::JumpIfFalseOrPop(_)
                | Self::JumpIfTrueOrPop(_)
                | Self::ForNext(_)
        )
    }

    /// The jump target, if this is a jump.
    pub fn jump_target(self) -> Option<u32> {
        match self {
            Self::Jump(t)
            | Self::JumpIfFalse(t)
            | Self::JumpIfTrue(t)
            | Self::JumpIfFalseOrPop(t)
            | Self::JumpIfTrueOrPop(t)
            | Self::ForNext(t) => Some(t),
            _ => None,
        }
    }

    /// True for opcodes that carry an inline-cache site id. LoadGlobal
    /// sites are tracked via the SiteTable (the opcode payload slot is
    /// the name index), so `site_id` reports None for it; the site's
    /// instr_idx identifies the LoadGlobal instruction.
    pub fn site_id(self) -> Option<u32> {
        match self {
            Self::LoadAttr { site, .. }
            | Self::CallFunction { site, .. }
            | Self::CallFunctionKw { site, .. } => Some(site),
            _ => None,
        }
    }

    /// Stable opcode name (disassembly + debug).
    pub fn name(self) -> &'static str {
        OPCODE_NAMES[self.opcode_index()]
    }

    /// Index into [`OPCODE_NAMES`] (discriminant-order position).
    pub fn opcode_index(self) -> usize {
        match self {
            Self::LoadConst(_) => 0,
            Self::LoadFast(_) => 1,
            Self::LoadGlobal(_) => 2,
            Self::StoreFast(_) => 3,
            Self::StoreGlobal(_) => 4,
            Self::PopTop => 5,
            Self::BinOp(_) => 6,
            Self::UnaryOp(_) => 7,
            Self::Compare(_) => 8,
            Self::Jump(_) => 9,
            Self::JumpIfFalse(_) => 10,
            Self::JumpIfTrue(_) => 11,
            Self::JumpIfFalseOrPop(_) => 12,
            Self::JumpIfTrueOrPop(_) => 13,
            Self::GetIter => 14,
            Self::ForNext(_) => 15,
            Self::CallFunction { .. } => 16,
            Self::LoadAttr { .. } => 17,
            Self::Subscript => 18,
            Self::StoreSubscript => 19,
            Self::LoadConstNone => 20,
            Self::LoadConstTrue => 21,
            Self::LoadConstFalse => 22,
            Self::LoadConstEmptyTuple => 23,
            Self::BuildTuple(_) => 24,
            Self::BuildList(_) => 25,
            Self::BuildDict(_) => 26,
            Self::BuildSet(_) => 27,
            Self::ReturnValue => 28,
            Self::ReturnNone => 29,
            Self::AugOp { .. } => 30,
            Self::LoadAttrCall { .. } => 31,
            Self::StoreAttr { .. } => 32,
            Self::CallFunctionKw { .. } => 33,
            Self::FallbackToTreeWalk => 34,
        }
    }
}

/// One name per opcode, discriminant order (kept in sync with
/// `Instr::opcode_index`; `decode::disassemble` and tests rely on it).
pub const OPCODE_NAMES: [&str; 35] = [
    "LoadConst",
    "LoadFast",
    "LoadGlobal",
    "StoreFast",
    "StoreGlobal",
    "PopTop",
    "BinOp",
    "UnaryOp",
    "Compare",
    "Jump",
    "JumpIfFalse",
    "JumpIfTrue",
    "JumpIfFalseOrPop",
    "JumpIfTrueOrPop",
    "GetIter",
    "ForNext",
    "CallFunction",
    "LoadAttr",
    "Subscript",
    "StoreSubscript",
    "LoadConstNone",
    "LoadConstTrue",
    "LoadConstFalse",
    "LoadConstEmptyTuple",
    "BuildTuple",
    "BuildList",
    "BuildDict",
    "BuildSet",
    "ReturnValue",
    "ReturnNone",
    "AugOp",
    "LoadAttrCall",
    "StoreAttr",
    "CallFunctionKw",
    "FallbackToTreeWalk",
];
