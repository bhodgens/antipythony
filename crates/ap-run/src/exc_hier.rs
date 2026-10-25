//! The v0 exception hierarchy (task brief §4): class names + subclass
//! relationships for raise/catch matching, and `str(exc)` rendering.
//!
//! The hierarchy is the task's pinned v0 set. Catch matching follows
//! CPython: `except E` matches when the raised class IS E or a (transitive)
//! subclass of E per this table. Chaining (`__cause__`/`__context__`) rides
//! on the exception instance (S-02-34); traceback TEXT rendering lives in
//! interp.rs's uncaught handler (C-25/S-06-7).

/// Parent chains for each v0 exception class (transitive closure is tiny).
/// Every name is its own subclass (match-self falls out).
pub fn is_subclass(class: &str, of: &str) -> bool {
    // Transitive walk: class -> ... -> of through PARENT edges.
    let mut cur = class;
    let mut hops = 0;
    loop {
        if cur == of {
            return true;
        }
        // 02.08-full part 1: io stream ABC relations (s005 pin:
        // issubclass(io.BytesIO, io.RawIOBase) et al).
        if io_subclass_edge(cur, of) {
            return true;
        }
        hops += 1;
        if hops > 8 {
            return false;
        }
        match parent_of(cur) {
            Some(p) => cur = p,
            None => return false,
        }
    }
}

/// io ABC edges: (subclass, base) pairs the io name surface needs.
fn io_subclass_edge(class: &str, of: &str) -> bool {
    matches!(
        (class, of),
        ("BytesIO", "BufferedIOBase")
            | ("BytesIO", "IOBase")
            | ("BufferedReader", "BufferedIOBase")
            | ("BufferedReader", "IOBase")
            | ("BufferedWriter", "BufferedIOBase")
            | ("BufferedWriter", "IOBase")
            | ("BufferedRWPair", "BufferedIOBase")
            | ("BufferedRWPair", "IOBase")
            | ("BufferedRandom", "BufferedIOBase")
            | ("BufferedRandom", "IOBase")
            | ("TextIOWrapper", "TextIOBase")
            | ("TextIOWrapper", "IOBase")
            | ("FileIO", "RawIOBase")
            | ("FileIO", "IOBase")
            | ("RawIOBase", "IOBase")
            | ("BufferedIOBase", "IOBase")
            | ("TextIOBase", "IOBase")
            | ("StringIO", "TextIOBase")
            | ("StringIO", "IOBase")
            | ("UnsupportedOperation", "OSError")
            | ("TemporaryDirectory", "object")
    )
}

/// Direct parent (None for BaseException).
fn parent_of(class: &str) -> Option<&'static str> {
    match class {
        "BaseException" => None,
        "KeyboardInterrupt" | "SystemExit" => Some("BaseException"),
        "Exception" => Some("BaseException"),
        "StopIteration" => Some("Exception"),
        "ArithmeticError" => Some("Exception"),
        "ZeroDivisionError" | "OverflowError" | "FloatingPointError" => Some("ArithmeticError"),
        // 02.08-lite stdlib exception types (json).
        "JSONDecodeError" => Some("ValueError"),
        "TypeError" | "ValueError" | "NameError" | "LookupError" | "AttributeError"
        | "RuntimeError" | "AssertionError" | "MemoryError" | "OSError" => Some("Exception"),
        "UnboundLocalError" => Some("NameError"),
        "IndexError" | "KeyError" => Some("LookupError"),
        "NotImplementedError" | "RecursionError" => Some("RuntimeError"),
        // 02.04a: ModuleNotFoundError ⊂ ImportError (file-import machinery).
        "ModuleNotFoundError" => Some("ImportError"),
        "UnicodeError" => Some("ValueError"),
        "UnicodeDecodeError" | "UnicodeEncodeError" => Some("UnicodeError"),
        "EOFError" => Some("OSError"),
        // 02.04 completion: the OSError family (gettext/os surfaces in the
        // unittest chain probe them by name).
        "FileNotFoundError"
        | "PermissionError"
        | "FileExistsError"
        | "NotADirectoryError"
        | "IsADirectoryError"
        | "InterruptedError"
        | "ProcessLookupError"
        | "TimeoutError"
        | "BlockingIOError"
        | "ChildProcessError"
        | "BrokenPipeError"
        | "ConnectionAbortedError"
        | "ConnectionRefusedError"
        | "ConnectionResetError"
        | "ConnectionError" => Some("OSError"),
        _ => Some("Exception"),
    }
}

#[allow(dead_code)]
fn is_subclass_table(class: &str, of: &str) -> bool {
    if class == of {
        return true;
    }
    match (class, of) {
        // Everything is a BaseException subclass except nothing.
        (_, "BaseException") => true,
        ("StopIteration", "Exception") => true,
        ("ZeroDivisionError", "ArithmeticError")
        | ("OverflowError", "ArithmeticError")
        | ("FloatingPointError", "ArithmeticError") => true,
        ("ArithmeticError", "Exception")
        | ("TypeError", "Exception")
        | ("ValueError", "Exception")
        | ("UnicodeError", "ValueError")
        | ("UnicodeDecodeError", "UnicodeError")
        | ("UnicodeEncodeError", "UnicodeError")
        | ("UnicodeDecodeError", "ValueError")
        | ("UnicodeEncodeError", "ValueError")
        | ("NameError", "Exception")
        | ("UnboundLocalError", "NameError")
        | ("IndexError", "LookupError")
        | ("KeyError", "LookupError")
        | ("LookupError", "Exception")
        | ("AttributeError", "Exception")
        | ("RuntimeError", "Exception")
        | ("NotImplementedError", "RuntimeError")
        | ("RecursionError", "RuntimeError")
        | ("AssertionError", "Exception")
        | ("MemoryError", "Exception")
        | ("OSError", "Exception")
        | ("EOFError", "OSError") => true,
        _ => false,
    }
}

/// A raised exception in flight: class name, args tuple, and chaining
/// slots. Traceback frames accumulate on the interpreter's current
/// exception while it propagates (C-25).
#[derive(Debug, Clone)]
pub struct ExceptionValue {
    pub class: String,
    pub args: Vec<ap_obj::value::Value>,
    /// `raise X from Y` — cause chain.
    pub cause: Option<Box<ExceptionValue>>,
    /// Implicit context: the exception being handled when this one raised.
    pub context: Option<Box<ExceptionValue>>,
    /// The real instance Value when the raise came from one (e007: `e is
    /// inst` identity preserved through the handler binding).
    pub instance: Option<ap_obj::value::Value>,
    /// Source line of the `raise` that created this exception (captured at
    /// the Raise arm from the current frame's line cell). The unraisable
    /// report uses it for the raise-site frame: by report time the
    /// finalizer's frame has already popped, so the frame cells no longer
    /// carry the raise site.
    pub raise_line: usize,
}

impl ExceptionValue {
    pub fn new(class: &str, args: Vec<ap_obj::value::Value>) -> Self {
        ExceptionValue {
            class: class.to_string(),
            args,
            cause: None,
            context: None,
            instance: None,
            raise_line: 0,
        }
    }

    /// Adapt a value-layer PyException (payload-only) into a full
    /// exception. The message becomes args[0] (a str) so `str(e)` renders
    /// byte-identically, EXCEPT KeyError whose str() is the repr of args[0]
    /// (S-01-24/V043) — the args value is already the key there.
    pub fn from_py(exc: ap_obj::exc::PyException) -> Self {
        let msg = exc.message;
        if exc.class == "MemoryError" && msg.is_empty() {
            return ExceptionValue::new("MemoryError", vec![]);
        }
        ExceptionValue::new(
            &exc.class,
            vec![ap_obj::Value::Str(ap_obj::PyString::new(msg))],
        )
    }

    /// One-arg class-message constructor used all over the executor.
    pub fn msg(class: &str, msg: impl Into<String>) -> Self {
        ExceptionValue::new(
            class,
            vec![ap_obj::Value::Str(ap_obj::PyString::new(msg.into()))],
        )
    }

    /// `str(exc)` (S-06-7: `ExceptionType: str(exc)` last line uses this).
    /// - zero args → ""
    /// - one arg → str(arg) (KeyError special: repr(arg))
    /// - multi-arg → "arg1, arg2" joined with ", " (str of each)
    pub fn py_str(&self) -> Result<String, ap_obj::exc::PyException> {
        // A-3 item 3: OSError(errno, strerror) formats as
        // "[Errno <errno>] <strerror>" in CPython (BaseException.__str__
        // special case for OSError with a 2-int-first args tuple).
        if self.class == "OSError"
            || self.class == "PermissionError"
            || self.class == "FileNotFoundError"
            || self.class == "ProcessLookupError"
        {
            if self.args.len() == 2 {
                if let Some(errno) = self.args[0].as_int().and_then(|n| n.to_i64()) {
                    let strerror = ap_obj::ops::to_str(&self.args[1])?;
                    return Ok(format!("[Errno {}] {}", errno, strerror));
                }
            }
            // A-3: 3-arg OSError(errno, strerror, filename) appends the path
            // (CPython: "[Errno 2] No such file or directory: '/x'").
            if self.args.len() == 3 {
                if let Some(errno) = self.args[0].as_int().and_then(|n| n.to_i64()) {
                    let strerror = ap_obj::ops::to_str(&self.args[1])?;
                    let fname = ap_obj::ops::repr(&self.args[2])?;
                    return Ok(format!("[Errno {}] {}: {}", errno, strerror, fname));
                }
            }
        }
        match self.args.len() {
            0 => Ok(String::new()),
            1 => {
                if self.class == "KeyError" {
                    ap_obj::ops::repr(&self.args[0])
                } else {
                    ap_obj::ops::to_str(&self.args[0])
                }
            }
            _ => {
                // CPython: BaseException.__str__ for multi-arg is str(args)
                // == the tuple repr (e001/e005 observed "('a', 2)").
                let t = ap_obj::value::Tuple(self.args.clone());
                ap_obj::ops::repr(&ap_obj::value::Value::Tuple(std::rc::Rc::new(t)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_matches() {
        assert!(is_subclass("ZeroDivisionError", "ArithmeticError"));
        assert!(is_subclass("ZeroDivisionError", "Exception"));
        assert!(is_subclass("ZeroDivisionError", "BaseException"));
        assert!(is_subclass("UnboundLocalError", "NameError"));
        assert!(is_subclass("KeyError", "Exception"));
        assert!(is_subclass("KeyError", "BaseException"));
        assert!(is_subclass("NotImplementedError", "RuntimeError"));
        assert!(is_subclass("RecursionError", "Exception"));
        assert!(is_subclass("KeyboardInterrupt", "BaseException"));
        assert!(!is_subclass("KeyboardInterrupt", "Exception"));
        assert!(!is_subclass("SystemExit", "Exception"));
        assert!(is_subclass("UnicodeDecodeError", "ValueError"));
        assert!(!is_subclass("ValueError", "TypeError"));
    }

    #[test]
    fn str_rendering() {
        let e = ExceptionValue::msg("ValueError", "boom");
        assert_eq!(e.py_str().unwrap(), "boom");
        let bare = ExceptionValue::new("MemoryError", vec![]);
        assert_eq!(bare.py_str().unwrap(), "");
        // KeyError str is the repr of the key.
        let ke = ExceptionValue::new(
            "KeyError",
            vec![ap_obj::Value::Str(ap_obj::PyString::new("zz"))],
        );
        assert_eq!(ke.py_str().unwrap(), "'zz'");
    }
}
