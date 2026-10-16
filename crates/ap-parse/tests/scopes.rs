//! Scope-classification tests for AST→IR lowering (leaf 01.03).
//!
//! Classifications per docs/spec/02-control-flow.md: S-02-1 (LEGB),
//! S-02-2 (any assignment => local), S-02-3 (global), S-02-4 (nonlocal),
//! S-02-5 (class bodies/comprehensions are NOT enclosing function scopes),
//! S-02-12/S-02-13 (cells shared with nested scopes).

use ap_parse::{lower_source, ScopeInfo};

fn scope_of(src: &str, path: &[&str]) -> ScopeInfo {
    let (_, scopes) = lower_source(src).expect("program must lower");
    let want: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    scopes
        .scopes
        .into_iter()
        .find(|s| s.path == want)
        .unwrap_or_else(|| panic!("scope {want:?} not found"))
}

#[test]
fn global_declaration_classifies_as_global() {
    // spec 02 S-02-3 (global name) — A02 example: bump() declares counter.
    let s = scope_of(
        "counter = 0\ndef bump():\n    global counter\n    counter += 1\n",
        &["bump"],
    );
    assert_eq!(s.globals, vec!["counter"]);
    assert!(s.cells.is_empty());
    assert!(s.frees.is_empty());
    assert!(s.nonlocals.is_empty());
}

#[test]
fn nonlocal_binds_enclosing_cell() {
    // spec 02 S-02-4/S-02-13: add()'s `nonlocal total` shares make_accumulator's cell.
    let add = scope_of(
        "def make_accumulator():\n    total = 0\n    def add(n):\n        nonlocal total\n        total += n\n        return total\n    return add\n",
        &["make_accumulator", "<locals>", "add"],
    );
    assert_eq!(add.nonlocals, vec!["total"]);
    // `total` rides in `nonlocals` (the declaration) — frees records only
    // *undeclared* captures so the two sets don't double-report.
    assert!(add.frees.is_empty());
    let outer = scope_of(
        "def make_accumulator():\n    total = 0\n    def add(n):\n        nonlocal total\n        total += n\n        return total\n    return add\n",
        &["make_accumulator"],
    );
    assert_eq!(outer.cells, vec!["total"]);
}

#[test]
fn free_read_through_function_scope() {
    // spec 02 S-02-12: inner reads outer's local without declaring nonlocal.
    let (inner, outer) = {
        let src = "def outer():\n    x = 1\n    def inner():\n        return x\n    return inner\n";
        (
            scope_of(src, &["outer", "<locals>", "inner"]),
            scope_of(src, &["outer"]),
        )
    };
    assert_eq!(inner.frees, vec!["x"]);
    assert_eq!(outer.cells, vec!["x"]);
}

#[test]
fn class_bodies_are_not_enclosing_scopes() {
    // spec 02 S-02-5: a method referencing a class-body name reads past it.
    let src = "class C:\n    y = 1\n    def m(self):\n        return y\n";
    let m = scope_of(src, &["C", "<locals>", "m"]);
    assert!(
        !m.frees.contains(&"y".to_string()),
        "class body names must not leak as method frees (S-02-5)"
    );
}

#[test]
fn comprehension_scope_is_own_scope() {
    // spec 02 S-02-5/S-02-2: the comprehension target binds in its own scope;
    // a read of an enclosing local from inside is a free var of the comp scope.
    let (_, scopes) = lower_source("def f(xs):\n    return [x for x in xs]\n").expect("lower");
    assert!(
        scopes
            .scopes
            .iter()
            .any(|s| s.path.last().map(|p| p == "<listcomp>").unwrap_or(false)),
        "comprehension scope must be classified separately"
    );
}

#[test]
fn assignment_makes_local_despite_module_binding() {
    // spec 02 S-02-2: `x = 0` inside f makes x LOCAL (shadows the global),
    // not a global read.
    let s = scope_of("x = 0\ndef f():\n    x = 1\n    return x\n", &["f"]);
    assert!(s.globals.is_empty());
    assert!(s.frees.is_empty());
}

#[test]
fn plain_global_read_is_not_free() {
    // A read with no local binding anywhere up to module scope is a global
    // read, not a closure free (S-02-1).
    let s = scope_of("g = 1\ndef f():\n    return g\n", &["f"]);
    assert!(s.frees.is_empty());
    assert!(s.cells.is_empty());
}

#[test]
fn scope_paths_are_sorted_and_unique() {
    let (_, scopes) =
        lower_source("def a():\n    def b():\n        pass\n    return b\ndef z():\n    pass\n")
            .expect("lower");
    let mut paths: Vec<Vec<String>> = scopes.scopes.iter().map(|s| s.path.clone()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
    paths.dedup();
    assert_eq!(paths.len(), sorted.len(), "no duplicate scope paths");
}

#[test]
fn scope_sets_are_sorted() {
    let (_, scopes) =
        lower_source("def f():\n    global z, a\n    z = 1\n    a = 2\n").expect("lower");
    let f = scopes
        .scopes
        .iter()
        .find(|s| s.path == vec!["f".to_string()])
        .expect("f scope");
    assert_eq!(f.globals, vec!["a", "z"], "globals sorted");
}

#[test]
fn lambda_scope_classified() {
    // Lambdas are function scopes with frame name <lambda> (S-02-38).
    let (_, scopes) =
        lower_source("def f():\n    y = 1\n    g = lambda: y\n    return g\n").expect("lower");
    let lam = scopes
        .scopes
        .iter()
        .find(|s| s.path.last().map(|p| p == "<lambda>").unwrap_or(false))
        .expect("lambda scope");
    assert_eq!(lam.frees, vec!["y"]);
}
