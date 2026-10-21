//! CPython-exact shortest round-trip float repr (S-01-41, C-15..C-17).
//!
//! Single source of truth for float rendering: `ap-ir`'s fold needs the same
//! bytes the runtime will print, so the function lives here and ap-ir should
//! (in a later cleanup) re-export rather than duplicate it.
//!
//! Algorithm: Rust's `{:?}` for f64 produces the shortest decimal string that
//! round-trips (Grisu3 + fallback), with the same digit selection as CPython's
//! David Gay `repr` — both pick the fewest-significant-digits representation,
//! ties to the closest value. The exponent-form thresholds differ per language
//! convention, so this module re-lays-out the digits to CPython's rules:
//!
//! - fixed notation for `1e-4 <= |x| < 1e16` (and their rounded digit strings
//!   that still land in the window after re-exponentiating);
//! - exponential `Xe±NN` (>= 2 exponent digits) otherwise;
//! - integral non-exponent values get a trailing `.0`;
//! - exponent-form mantissa drops a trailing `.0`;
//! - specials: `inf` / `-inf` / `nan`; `-0.0` keeps its sign.
//!
//! The fixed/exponential decision is made on the ROUND-TRIPPED value's
//! decimal exponent (as CPython does: `repr(10000000000000001.0)` → `1e+16`
//! because the shortest digits are `1e16`), not on the raw f64 magnitude —
//! this is why `1e16` prints exponent-form while `9999999999999998.0` stays
//! fixed.

/// Python `repr(float)` / `str(float)` (identical in 3.x), byte-exact vs
/// CPython 3.14 for every finite and special value (C-15).
pub fn py_float_repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }
    let rust = format!("{v:?}"); // shortest round-trip digits
    let (mant, exp) = match rust.split_once('e') {
        Some((m, e)) => (m.to_string(), Some(e.parse::<i32>().unwrap())),
        None => (rust, None),
    };
    // Decimal exponent of the value under Python's layout convention: for
    // `d.ddd` mantissa with exponent e, Python uses exponent form iff
    // e < -4 or e >= 16.
    let exp10 = match exp {
        Some(e) => e,
        None => {
            // Fixed form from Rust; compute the decimal exponent from the
            // integer part's length. The leading '-' must NOT count toward
            // the digit length (F0402-1: repr(-1e15) computed exp10=16 from
            // "-1000000000000000".len()==17 and rendered '-1000000000000000e+16'
            // instead of '-1000000000000000.0').
            let int_part = mant.split('.').next().unwrap_or("0");
            let digits = int_part.strip_prefix('-').unwrap_or(int_part);
            if digits == "0" {
                // 0.00123 / -0.00123 style: count leading zeros after "0."
                let frac = mant.split('.').nth(1).unwrap_or("");
                -((frac.len() - frac.trim_start_matches('0').len()) as i32) - 1
            } else {
                (digits.len() as i32) - 1
            }
        }
    };
    if exp10 < -4 || exp10 >= 16 {
        // exponent form: strip trailing ".0" from mantissa, two-digit exp
        let m = mant.strip_suffix(".0").unwrap_or(&mant);
        let (sign, mag) = if exp10 < 0 {
            ("-", -exp10)
        } else {
            ("+", exp10)
        };
        format!("{m}e{sign}{mag:02}")
    } else if mant.contains('.') {
        mant
    } else {
        format!("{mant}.0")
    }
}

#[cfg(test)]
mod tests {
    use super::py_float_repr as r;

    #[test]
    fn cpython_spot_table() {
        // All bytes below probed against /opt/homebrew/bin/python3.14 3.14.7
        assert_eq!(r(0.1), "0.1");
        assert_eq!(r(1e16), "1e+16");
        assert_eq!(r(1e15), "1000000000000000.0");
        assert_eq!(r(1e-7), "1e-07");
        assert_eq!(r(1.5e308), "1.5e+308");
        assert_eq!(r(f64::INFINITY), "inf");
        assert_eq!(r(f64::NEG_INFINITY), "-inf");
        assert_eq!(r(f64::NAN), "nan");
        assert_eq!(r(-0.0), "-0.0");
        assert_eq!(r(5e-324), "5e-324"); // smallest subnormal
        assert_eq!(r(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(r(10.0 / 3.0), "3.3333333333333335");
        assert_eq!(r(1.0), "1.0");
        assert_eq!(r(0.30000000000000004), "0.30000000000000004");
        assert_eq!(r(1e-4), "0.0001");
        assert_eq!(r(3.5e-5), "3.5e-05");
        assert_eq!(r(12345678.0), "12345678.0");
        assert_eq!(r(9999999999999998.0), "9999999999999998.0");
        assert_eq!(r(10000000000000001.0), "1e+16"); // shortest digits → 1e16
        assert_eq!(r(1.7976931348623157e308), "1.7976931348623157e+308");
        assert_eq!(r(2.2250738585072014e-308), "2.2250738585072014e-308");
        assert_eq!(r(1e-320), "1e-320"); // subnormal
        assert_eq!(r(1.23e45), "1.23e+45");
        assert_eq!(r(6.02e23), "6.02e+23");
        assert_eq!(r(0.15625), "0.15625");
        assert_eq!(r(0.1 + 0.7), "0.7999999999999999");
        assert_eq!(r(2.0f64.sqrt()), "1.4142135623730951");
        assert_eq!(r(123456789.123456789_f64), "123456789.12345679");
        assert_eq!(r(1e20), "1e+20");
        assert_eq!(r(2.5e-10), "2.5e-10");
    }
}
