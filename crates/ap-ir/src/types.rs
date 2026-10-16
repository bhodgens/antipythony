//! Identifier and lattice types for ap-ir.

use std::fmt;

/// Opaque reference to a definition (function or class) elsewhere in the module.
///
/// v0: a plain index assigned by the frontend; the verifier (01.04) and the
/// backends resolve it. Not order-stable across lowering changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(pub u32);

/// Type lattice for specialization (orchestrator IR shape, v0).
///
/// Annotations seed it (01.03); tier-1 feedback refines it (Plan 02). This is
/// a *static guess*, never a contract: every backend MUST treat `Unknown` as
/// "anything" and must not specialize away the general path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeLattice {
    /// No information. The only sound default.
    Unknown,
    /// `int` (arbitrary precision — compat contract C-12; spec 01 S-01-1).
    Int,
    /// `float` (IEEE-754 binary64 — spec 01 S-01-8).
    Float,
    /// `str` (immutable Unicode sequence — spec 01 S-01-14).
    Str,
    /// `bool` (a subtype of `int` — spec 01 S-01-9).
    Bool,
    /// The `None` singleton.
    None,
    /// Exactly this class, no subclass allowance (e.g. a fresh local class).
    ExactClass(DefId),
}

impl fmt::Display for TypeLattice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeLattice::Unknown => write!(f, "unknown"),
            TypeLattice::Int => write!(f, "int"),
            TypeLattice::Float => write!(f, "float"),
            TypeLattice::Str => write!(f, "str"),
            TypeLattice::Bool => write!(f, "bool"),
            TypeLattice::None => write!(f, "none"),
            TypeLattice::ExactClass(id) => write!(f, "class#{}", id.0),
        }
    }
}

/// A Python identifier (name as written in source; never empty).
pub type Name = String;
