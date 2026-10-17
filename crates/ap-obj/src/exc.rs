//! Minimal exception value for the pure value layer (pre-02.02).
//!
//! 02.01's operations need to report failures with CPython's exact
//! `type: message` text (S-01-8 division text, S-01-11 conversion text,
//! S-01-24 unhashable-key text, C-14 overflow/memory text, C-16 amendment
//! A-1 message unification). The full exception machinery (tracebacks,
//! chaining, `raise`/`except`) is 02.02's leaf; this is the payload.

/// A raised Python exception: class name + message, both byte-exact strings.
///
/// The message NEVER carries the trailing newline; renderers add it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyException {
    /// Exception class name, e.g. `"ZeroDivisionError"` (no module prefix —
    /// builtins only in the v1 value layer).
    pub class: &'static str,
    /// Message exactly as CPython renders it in `str(exc)`. Empty string when
    /// CPython shows none (e.g. bare `MemoryError`).
    pub message: String,
}

impl PyException {
    pub fn new(class: &'static str, message: impl Into<String>) -> Self {
        PyException {
            class,
            message: message.into(),
        }
    }

    /// `ZeroDivisionError: division by zero` — one text for every operand
    /// type pair (compat contract amendment A-1, C-16; NOT the pre-3.14
    /// "float division"/"float modulo" wording).
    pub fn zero_division() -> Self {
        PyException::new("ZeroDivisionError", "division by zero")
    }

    /// `ZeroDivisionError: zero to a negative power` (S-01-2).
    pub fn zero_to_negative_power() -> Self {
        PyException::new("ZeroDivisionError", "zero to a negative power")
    }

    /// `ZeroDivisionError: 0.0 cannot be raised to a negative power` — the
    /// float-base variant CPython raises for `0.0 ** -1.0`.
    pub fn float_zero_to_negative_power() -> Self {
        PyException::new(
            "ZeroDivisionError",
            "0.0 cannot be raised to a negative power",
        )
    }

    /// `OverflowError: int too large to convert to float` (S-01-11, C-14).
    pub fn int_too_large_for_float() -> Self {
        PyException::new("OverflowError", "int too large to convert to float")
    }

    /// `ValueError: cannot convert float NaN to integer` (S-01-11).
    pub fn nan_to_int() -> Self {
        PyException::new("ValueError", "cannot convert float NaN to integer")
    }

    /// `OverflowError: cannot convert float infinity to integer` (S-01-11).
    pub fn inf_to_int() -> Self {
        PyException::new("OverflowError", "cannot convert float infinity to integer")
    }

    /// `OverflowError: integer division result too large for a float` —
    /// int `/` int whose true quotient exceeds float range (C-14, probed
    /// live: `10**400 / 1`).
    pub fn int_div_too_large_for_float() -> Self {
        PyException::new(
            "OverflowError",
            "integer division result too large for a float",
        )
    }

    /// `TypeError: unhashable type: '<type>'` (S-01-39).
    pub fn unhashable(type_name: &str) -> Self {
        PyException::new("TypeError", format!("unhashable type: '{type_name}'"))
    }

    /// `TypeError: cannot use '<type>' as a dict key (unhashable type:
    /// '<type>')` — the dict-key phrasing (S-01-24), type named twice.
    pub fn unhashable_dict_key(type_name: &str) -> Self {
        PyException::new(
            "TypeError",
            format!("cannot use '{type_name}' as a dict key (unhashable type: '{type_name}')"),
        )
    }

    /// `TypeError: cannot use '<type>' as a set element (unhashable type:
    /// '<type>')` — the set-element phrasing (S-01-26), distinct from
    /// [`PyException::unhashable_dict_key`].
    pub fn unhashable_set_element(type_name: &str) -> Self {
        PyException::new(
            "TypeError",
            format!("cannot use '{type_name}' as a set element (unhashable type: '{type_name}')"),
        )
    }

    /// C-14/A-2: `str(int)` over the conversion digit limit — NO digit count
    /// in the text. `limit` is `sys.get_int_max_str_digits()` (4300 default).
    pub fn int_str_limit(limit: usize) -> Self {
        PyException::new(
            "ValueError",
            format!(
                "Exceeds the limit ({limit} digits) for integer string conversion; \
                 use sys.set_int_max_str_digits() to increase the limit"
            ),
        )
    }

    /// C-14/A-2: `int(str)` over the limit — WITH the actual input digit
    /// count (`: value has N digits` inserted before `; use`).
    pub fn str_int_limit(limit: usize, actual_digits: usize) -> Self {
        PyException::new(
            "ValueError",
            format!(
                "Exceeds the limit ({limit} digits) for integer string conversion: \
                 value has {actual_digits} digits; \
                 use sys.set_int_max_str_digits() to increase the limit"
            ),
        )
    }

    /// `ValueError: invalid literal for int() with base 10: '<s>'` (S-01-13).
    /// The literal is repr'd (single quotes, no escaping of the raw text —
    /// CPython embeds it verbatim between quotes).
    pub fn invalid_int_literal(literal: &str) -> Self {
        PyException::new(
            "ValueError",
            format!("invalid literal for int() with base 10: '{literal}'"),
        )
    }
}

impl std::fmt::Display for PyException {
    /// `Class: message` — the last-traceback-line form. Empty message renders
    /// as bare `Class` (bare `MemoryError` per C-14 probe).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.class)
        } else {
            write!(f, "{}: {}", self.class, self.message)
        }
    }
}
