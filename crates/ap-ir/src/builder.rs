//! Clause-cited construction API for the IR.
//!
//! Every method's doc comment cites the compat-contract clause (C-xx) or
//! spec-pack clause (S-xx-yy) whose semantics the constructed node preserves.
//! One-line citations; the normative text lives in `docs/compat-contract.md`
//! and `docs/spec/`.
//!
//! The Builder is deliberately thin: it panics on *structural* misuse (an
//! empty compare chain) and never on semantic questions (it does not evaluate
//! anything).

use crate::expr::{
    Arg, BinOp, BoolOp, CmpOp, CompClause, Conversion, DictItem, Expr, FStrPart, LambdaSig, UnaryOp,
};
use crate::item::{
    Ann, ClassDef, ExceptHandler, FnBody, FnDef, ImportAlias, MatchCase, Param, ParamKind, Stmt,
    Target, TryStmt, WithItem,
};
use crate::module::Module;
use crate::types::{DefId, Name, TypeLattice};

/// Constructor facade for IR nodes.
///
/// All methods are free of side effects and just assemble the node, applying
/// the structural invariants the node kind requires (e.g. non-empty compare
/// chains).
#[derive(Debug, Default)]
pub struct Builder {
    /// Next `DefId` to hand out for classes.
    next_def: u32,
}

impl Builder {
    /// A fresh builder.
    pub fn new() -> Self {
        Builder { next_def: 0 }
    }

    // ---- literals -------------------------------------------------------

    /// Integer literal (value must fit; else `int_big_lit`).
    /// Clauses: C-12, C-13 (arbitrary precision, no wraparound).
    pub fn int_lit(&self, v: i128) -> Expr {
        Expr::LitInt(v)
    }

    /// Integer literal beyond `i128`, digits preserved exactly.
    /// Clauses: C-12, C-13.
    pub fn int_big_lit(&self, digits: impl Into<String>) -> Expr {
        Expr::LitBigInt(digits.into())
    }

    /// Float literal. Clauses: C-16, S-01-8 (IEEE-754 binary64).
    pub fn float_lit(&self, v: f64) -> Expr {
        Expr::LitFloat(v)
    }

    /// Complex literal. Clause: S-01-8 area (numeric tower).
    pub fn complex_lit(&self, re: f64, im: f64) -> Expr {
        Expr::LitComplex(re, im)
    }

    /// String literal (already unescaped; concatenation done by lowering).
    /// Clauses: S-01-14, C-18.
    pub fn str_lit(&self, s: impl Into<String>) -> Expr {
        Expr::LitStr(s.into())
    }

    /// Bytes literal. Clauses: S-01-17, S-01-18.
    pub fn bytes_lit(&self, b: impl Into<Vec<u8>>) -> Expr {
        Expr::LitBytes(b.into())
    }

    /// `True`/`False`. Clauses: S-01-9, S-01-10.
    pub fn bool_lit(&self, v: bool) -> Expr {
        Expr::LitBool(v)
    }

    /// `None`. Clause: S-01-27.
    pub fn none_lit(&self) -> Expr {
        Expr::LitNone
    }

    /// `...`. Clause: S-01-26 area (singletons).
    pub fn ellipsis_lit(&self) -> Expr {
        Expr::LitEllipsis
    }

    // ---- names and operators --------------------------------------------

    /// Name read. Clauses: S-02-1 (LEGB), S-02-2 (local classification).
    pub fn name(&self, n: impl Into<String>) -> Expr {
        Expr::Name(n.into())
    }

    /// `a op b`. Clauses: C-12..C-14, C-16, S-01-3, S-01-4, S-01-6, S-01-7, S-03-25.
    pub fn bin_op(&self, op: BinOp, a: Expr, b: Expr) -> Expr {
        Expr::BinOp(op, Box::new(a), Box::new(b))
    }

    /// `op x`. Clauses: S-01-1 (`~x == -x-1`), S-01-8.
    pub fn unary_op(&self, op: UnaryOp, x: Expr) -> Expr {
        Expr::UnaryOp(op, Box::new(x))
    }

    /// `a and b` / `a or b` (2+ operands). Clauses: S-01-27..S-01-30.
    ///
    /// # Panics
    /// With fewer than 2 operands (Python syntax requires 2+).
    pub fn bool_op(&self, op: BoolOp, operands: Vec<Expr>) -> Expr {
        assert!(operands.len() >= 2, "and/or need at least 2 operands");
        Expr::BoolOp(op, operands)
    }

    /// Chained comparison `a0 <op1> a1 <op2> ...`.
    /// Clause: S-01-31 (each middle evaluated once; short-circuit).
    ///
    /// # Panics
    /// Unless `ops.len() == operands.len() - 1` and both are non-empty.
    pub fn compare(&self, ops: Vec<CmpOp>, operands: Vec<Expr>) -> Expr {
        assert!(!ops.is_empty(), "comparison chain cannot be empty");
        assert_eq!(
            ops.len() + 1,
            operands.len(),
            "compare needs n ops and n+1 operands"
        );
        Expr::Compare(ops, operands)
    }

    // ---- calls, attributes, containers ----------------------------------

    /// `f(args)`. Clauses: S-02-7..S-02-10, S-03-28.
    pub fn call(&self, f: Expr, args: Vec<Arg>) -> Expr {
        Expr::Call(Box::new(f), args)
    }

    /// Positional call argument.
    pub fn arg_pos(&self, e: Expr) -> Arg {
        Arg::Pos(e)
    }

    /// `*xs` call argument. Clause: S-02-9.
    pub fn arg_star(&self, e: Expr) -> Arg {
        Arg::Star(e)
    }

    /// `k=v` call argument. Clause: S-02-8.
    pub fn arg_kw(&self, k: impl Into<String>, e: Expr) -> Arg {
        Arg::Kw(k.into(), e)
    }

    /// `**ks` call argument. Clause: S-02-9.
    pub fn arg_kwstar(&self, e: Expr) -> Arg {
        Arg::KwStar(e)
    }

    /// `obj.attr`. Clause: S-03-9.
    pub fn attribute(&self, obj: Expr, attr: impl Into<String>) -> Expr {
        Expr::Attribute(Box::new(obj), attr.into())
    }

    /// `obj[key]`. Clause: S-01-15.
    pub fn subscript(&self, obj: Expr, key: Expr) -> Expr {
        Expr::Subscript(Box::new(obj), Box::new(key))
    }

    /// `x[i:j:k]` slice object. Clause: S-01-15.
    pub fn slice(&self, lo: Option<Expr>, hi: Option<Expr>, step: Option<Expr>) -> Expr {
        Expr::Slice(lo.map(Box::new), hi.map(Box::new), step.map(Box::new))
    }

    /// `[..]`. Clause: S-01-19.
    pub fn list(&self, items: Vec<Expr>) -> Expr {
        Expr::List(items)
    }

    /// `(...)`. Clause: S-01-20.
    pub fn tuple(&self, items: Vec<Expr>) -> Expr {
        Expr::Tuple(items)
    }

    /// `{..}`. Clauses: S-01-25.
    pub fn set(&self, items: Vec<Expr>) -> Expr {
        Expr::Set(items)
    }

    /// `{k: v, ...}`. Clauses: S-01-21..S-01-24.
    pub fn dict(&self, items: Vec<DictItem>) -> Expr {
        Expr::Dict(items)
    }

    /// One `k: v` dict item.
    pub fn dict_item(&self, k: Expr, v: Expr) -> DictItem {
        DictItem { key: k, value: v }
    }

    /// `**spam` in a dict display. Clause: S-01-21.
    pub fn dict_expand(&self, e: Expr) -> Expr {
        Expr::DictExpand(Box::new(e))
    }

    // ---- functions, comprehensions, generators --------------------------

    /// `lambda params: body`. Clauses: S-02-8, S-02-12, S-02-38.
    pub fn lambda(&self, params: Vec<Param>, body: Expr) -> Expr {
        Expr::Lambda(Box::new(LambdaSig { params }), Box::new(body))
    }

    /// `[e for t in it if ..]`. Clauses: S-02-2 (own scope), S-02-17 per clause.
    pub fn list_comp(&self, elt: Expr, clauses: Vec<CompClause>) -> Expr {
        Expr::ListComp(Box::new(elt), clauses)
    }

    /// `{e for t in it if ..}`. Clauses: S-02-2, S-01-25.
    pub fn set_comp(&self, elt: Expr, clauses: Vec<CompClause>) -> Expr {
        Expr::SetComp(Box::new(elt), clauses)
    }

    /// `{k: v for t in it if ..}`. Clauses: S-02-2, S-01-21.
    pub fn dict_comp(&self, key: Expr, val: Expr, clauses: Vec<CompClause>) -> Expr {
        Expr::DictComp(Box::new(key), Box::new(val), clauses)
    }

    /// `(e for t in it if ..)`. Clauses: S-02-20..S-02-25.
    pub fn generator(&self, elt: Expr, clauses: Vec<CompClause>) -> Expr {
        Expr::Generator(Box::new(elt), clauses)
    }

    /// One comprehension clause.
    pub fn comp_clause(&self, target: Expr, iter: Expr, ifs: Vec<Expr>) -> CompClause {
        CompClause { target, iter, ifs }
    }

    /// `yield [e]`. Clauses: S-02-21, S-02-24, S-02-25.
    pub fn yield_(&self, e: Option<Expr>) -> Expr {
        Expr::Yield(e.map(Box::new))
    }

    /// `yield from sub`. Clause: S-02-26.
    pub fn yield_from(&self, e: Expr) -> Expr {
        Expr::YieldFrom(Box::new(e))
    }

    /// `await e`. Clause: S-05-13 area.
    pub fn await_(&self, e: Expr) -> Expr {
        Expr::Await(Box::new(e))
    }

    // ---- f-strings, conditionals, starred, named -------------------------

    /// f-string from parts. Clauses: S-01-16, C-18.
    pub fn fstring(&self, parts: Vec<FStrPart>) -> Expr {
        Expr::FString(parts)
    }

    /// Literal f-string chunk.
    pub fn fstr_lit(&self, s: impl Into<String>) -> FStrPart {
        FStrPart::Lit(s.into())
    }

    /// `{expr}` chunk with optional conversion and spec (non-debug form).
    pub fn fstr_interp(
        &self,
        expr: Expr,
        conv: Option<Conversion>,
        spec: Option<Vec<FStrPart>>,
    ) -> FStrPart {
        FStrPart::Interp {
            expr,
            conv,
            spec,
            debug_text: None,
        }
    }

    /// `{expr=}` debug-form chunk: the literal source text `debug_text` (e.g.
    /// `"x = "`) is emitted before the converted value at render time.
    pub fn fstr_interp_debug(
        &self,
        expr: Expr,
        conv: Option<Conversion>,
        spec: Option<Vec<FStrPart>>,
        debug_text: impl Into<String>,
    ) -> FStrPart {
        FStrPart::Interp {
            expr,
            conv,
            spec,
            debug_text: Some(debug_text.into()),
        }
    }

    /// `c if t else e`. Clause: S-01-27 (truthiness; lazy branches).
    pub fn if_exp(&self, cond: Expr, then: Expr, els: Expr) -> Expr {
        Expr::IfExp(Box::new(cond), Box::new(then), Box::new(els))
    }

    /// `*x`. Clause: S-02-9.
    pub fn starred(&self, e: Expr) -> Expr {
        Expr::Starred(Box::new(e))
    }

    /// `t := v`. Clause: S-02-2 (binds in enclosing scope).
    pub fn named(&self, target: Expr, value: Expr) -> Expr {
        Expr::Named(Box::new(target), Box::new(value))
    }

    // ---- numbers by runtime shape (matrix "num_lit") ---------------------

    /// Number literal dispatched on Python's shape (int/float/complex read
    /// from the literal itself).
    /// Clauses: C-12/C-13 (int), C-16 (float), S-01-8.
    pub fn num_lit(&self, n: NumberLit) -> Expr {
        match n {
            NumberLit::Int(v) => self.int_lit(v),
            NumberLit::Big(digits) => self.int_big_lit(digits),
            NumberLit::Float(v) => self.float_lit(v),
            NumberLit::Complex(re, im) => self.complex_lit(re, im),
        }
    }

    // ---- defs ------------------------------------------------------------

    /// Allocate a fresh [`DefId`] (for classes; also usable for hidden
    /// comprehension/generator functions in 01.03).
    pub fn fresh_def(&mut self) -> DefId {
        let id = DefId(self.next_def);
        self.next_def += 1;
        id
    }

    /// `def f(params) -> ann: body`.
    /// Clauses: S-02-7 (def-time evaluation order), S-02-8, S-02-38.
    pub fn fn_def(
        &self,
        name: impl Into<String>,
        params: Vec<Param>,
        return_ann: Option<Ann>,
        decorators: Vec<Expr>,
        body: FnBody,
    ) -> FnDef {
        let is_generator = crate::walk::body_yields(&body);
        let ret_ty = return_ann
            .as_ref()
            .map(|a| a.ty.clone())
            .unwrap_or(TypeLattice::Unknown);
        FnDef {
            name: name.into(),
            params,
            return_ann,
            decorators,
            body,
            is_generator,
            is_async: false,
            ret_ty,
        }
    }

    /// Mark a fn def as `async def`. Clause: S-05-13 area.
    pub fn fn_async(mut f: FnDef) -> FnDef {
        f.is_async = true;
        f
    }

    /// One parameter.
    /// Clauses: S-02-8 (kinds/order), S-02-7 (defaults at def time).
    pub fn param(&self, name: impl Into<String>, kind: ParamKind) -> Param {
        Param {
            name: name.into(),
            kind,
            default: None,
            ann: None,
            ty: TypeLattice::Unknown,
        }
    }

    /// Set a parameter default (builder-style).
    pub fn with_default(mut p: Param, d: Expr) -> Param {
        p.default = Some(d);
        p
    }

    /// Set a parameter annotation, seeding the lattice when recognizable.
    pub fn with_ann(mut p: Param, ann: Ann) -> Param {
        p.ty = ann.ty.clone();
        p.ann = Some(ann);
        p
    }

    /// An annotation with a lattice guess.
    /// Clause: S-04 area (annotations bind without runtime effect in v1 subset).
    pub fn ann(&self, expr: Expr, ty: TypeLattice) -> Ann {
        Ann { expr, ty }
    }

    /// `class C(bases..., metaclass=M): body`.
    /// Clauses: S-03-1..S-03-4 (creation), S-03-15 (MRO computed at creation).
    pub fn class_def(
        &mut self,
        name: impl Into<String>,
        bases: Vec<Expr>,
        metaclass: Option<Expr>,
        decorators: Vec<Expr>,
        body: Vec<Stmt>,
    ) -> ClassDef {
        ClassDef {
            name: name.into(),
            bases,
            metaclass,
            decorators,
            body,
            id: self.fresh_def(),
        }
    }

    // ---- statements -------------------------------------------------------

    /// `targets = v`. Clause: S-02-19 (unpacking order; errors).
    ///
    /// # Panics
    /// With an empty target list (Python requires one).
    pub fn assign(&self, targets: Vec<Target>, value: Expr) -> Stmt {
        assert!(!targets.is_empty(), "assignment needs a target");
        Stmt::Assign(targets, value)
    }

    /// `x op= v`. Clauses: S-03-26 (inplace first), S-02-19.
    pub fn aug_assign(&self, target: Target, op: crate::expr::BinOp, value: Expr) -> Stmt {
        Stmt::AugAssign(target, op, value)
    }

    /// `x: T = v` / `x: T`. Clause: S-04 area (`__annotations__` binding).
    pub fn ann_assign(&self, target: Target, ann: Ann, value: Option<Expr>) -> Stmt {
        Stmt::AnnAssign(target, ann, value)
    }

    /// `return [e]`. Clause: S-02-11.
    pub fn return_(&self, e: Option<Expr>) -> Stmt {
        Stmt::Return(e)
    }

    /// `if c: then [else: els]` (elif chains nest naturally).
    /// Clause: S-02-15.
    pub fn if_(&self, cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
        Stmt::If(cond, then, els)
    }

    /// `while c: body [else: els]`. Clause: S-02-16.
    pub fn while_(&self, cond: Expr, body: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
        Stmt::While(cond, body, els)
    }

    /// `for t in it: body [else: els]`. Clause: S-02-17.
    pub fn for_(&self, target: Target, iter: Expr, body: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
        Stmt::For(target, iter, body, els)
    }

    /// `with items: body`. Clauses: S-02-40..S-02-42.
    pub fn with_(&self, items: Vec<WithItem>, body: Vec<Stmt>) -> Stmt {
        assert!(!items.is_empty(), "with needs at least one item");
        Stmt::With(items, body)
    }

    /// One with item.
    pub fn with_item(&self, expr: Expr, target: Option<Target>) -> WithItem {
        WithItem { expr, target }
    }

    /// try statement from parts. Clauses: S-02-30..S-02-34.
    pub fn try_(
        &self,
        body: Vec<Stmt>,
        handlers: Vec<ExceptHandler>,
        orelse: Vec<Stmt>,
        finalbody: Vec<Stmt>,
    ) -> Stmt {
        assert!(
            !handlers.is_empty() || !finalbody.is_empty(),
            "try needs except or finally"
        );
        Stmt::Try(TryStmt {
            body,
            handlers,
            orelse,
            finalbody,
        })
    }

    /// One except handler.
    pub fn except_handler(
        &self,
        ty: Option<Expr>,
        name: Option<String>,
        body: Vec<Stmt>,
    ) -> ExceptHandler {
        ExceptHandler { ty, name, body }
    }

    /// `raise [e [from cause]]`. Clauses: S-02-27, S-02-28, S-02-34.
    pub fn raise_(&self, e: Option<Expr>, cause: Option<Expr>) -> Stmt {
        Stmt::Raise(e, cause)
    }

    /// `match subj: cases`. Clause: S-02-35 area (uncaught format applies to
    /// match-raised errors); pattern semantics per PEP 634.
    pub fn match_(&self, subject: Expr, cases: Vec<MatchCase>) -> Stmt {
        assert!(!cases.is_empty(), "match needs at least one case");
        Stmt::Match(subject, cases)
    }

    /// `assert t [, msg]`. Clauses: S-02-39.
    pub fn assert_(&self, test: Expr, msg: Option<Expr>) -> Stmt {
        Stmt::Assert(test, msg)
    }

    /// `global a, b`. Clause: S-02-3.
    pub fn global_(&self, names: Vec<impl Into<String>>) -> Stmt {
        Stmt::Global(names.into_iter().map(Into::into).collect())
    }

    /// `nonlocal a, b`. Clause: S-02-4.
    pub fn nonlocal(&self, names: Vec<impl Into<String>>) -> Stmt {
        Stmt::Nonlocal(names.into_iter().map(Into::into).collect())
    }

    /// `import a.b.c [as d], ...`. Clauses: S-04-6, S-04-8, S-04-9, C-07.
    pub fn import_(&self, aliases: Vec<ImportAlias>) -> Stmt {
        assert!(!aliases.is_empty());
        Stmt::Import(aliases)
    }

    /// `from m import ...` / `from m import *`.
    /// Clauses: S-04-7, S-04-10, S-04-15.
    pub fn import_from(
        &self,
        module: impl Into<String>,
        aliases: Vec<ImportAlias>,
        star: bool,
    ) -> Stmt {
        if star {
            assert!(aliases.is_empty(), "star import takes no aliases");
        } else {
            assert!(!aliases.is_empty(), "non-star from-import needs aliases");
        }
        Stmt::ImportFrom(module.into(), aliases, star)
    }

    /// One import alias (`path` = dotted path).
    pub fn import_alias(&self, dotted: &str, alias: Option<impl Into<String>>) -> ImportAlias {
        ImportAlias {
            path: dotted.split('.').map(str::to_string).collect(),
            alias: alias.map(Into::into),
        }
    }

    /// `del targets`. Clauses: S-02-6, S-03-11.
    pub fn del_(&self, targets: Vec<Target>) -> Stmt {
        assert!(!targets.is_empty());
        Stmt::Del(targets)
    }

    /// `pass`.
    pub fn pass_(&self) -> Stmt {
        Stmt::Pass
    }

    /// Expression evaluated for effect.
    pub fn expr_stmt(&self, e: Expr) -> Stmt {
        Stmt::ExprStmt(e)
    }

    /// `break`. Clause: S-02-18.
    pub fn break_(&self) -> Stmt {
        Stmt::Break
    }

    /// `continue`. Clause: S-02-18.
    pub fn continue_(&self) -> Stmt {
        Stmt::Continue
    }

    // ---- module ----------------------------------------------------------

    /// Assemble a module from items.
    pub fn module(&self, items: Vec<crate::item::Item>) -> Module {
        Module { items }
    }

    // ---- targets ---------------------------------------------------------

    /// Name assignment target.
    pub fn target_name(&self, n: impl Into<String>) -> Target {
        Target::Name(n.into())
    }

    /// Subscript target.
    pub fn target_subscript(&self, obj: Expr, key: Expr) -> Target {
        Target::Subscript(Box::new(obj), Box::new(key))
    }

    /// Attribute target.
    pub fn target_attribute(&self, obj: Expr, attr: impl Into<String>) -> Target {
        Target::Attribute(Box::new(obj), attr.into())
    }

    /// Tuple/list unpacking target. Clause: S-02-19.
    pub fn target_tuple(&self, items: Vec<Target>) -> Target {
        Target::Tuple(items)
    }

    /// Starred target. Clause: S-02-19.
    pub fn target_starred(&self, e: Expr) -> Target {
        Target::Starred(Box::new(e))
    }
}

/// Number literal shape for [`Builder::num_lit`].
#[derive(Debug, Clone, PartialEq)]
pub enum NumberLit {
    /// Fits `i128`.
    Int(i128),
    /// Bigger — decimal digits.
    Big(String),
    /// Float.
    Float(f64),
    /// Complex.
    Complex(f64, f64),
}

/// Convenience re-export so the vtable test in `matrix` tests can name [`Name`].
#[allow(dead_code)]
type _NameUsed = Name;
