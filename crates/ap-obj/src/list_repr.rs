//! List: insertion-ordered mutable sequence (S-01-19) with CPython's
//! over-allocation growth. The type body lives in `value.rs` next to `Value`
//! (its `fmt_repr` is part of the Value recursion); this module re-exports
//! it under the canonical path so `ap_obj::List` reads naturally in 02.02.

pub use crate::value::PyList as List;
