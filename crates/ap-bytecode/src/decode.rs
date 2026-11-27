//! Textual disassembly (tests + APX_BP_DEBUG dumps). Not a semantic API.

use crate::code::CodeObject;
use crate::instr::Instr;

/// Render one instruction with its tables.
pub fn render_instr(code: &CodeObject, idx: usize) -> String {
    let ins = code.instrs[idx];
    let line = code.lines.line_for(idx as u32);
    let base = format!("{idx:4} L{line:<4} {:<18}", ins.name());
    match ins {
        Instr::LoadConst(c) => format!("{base}{}  # {}", c, code.consts.render(c)),
        Instr::LoadFast(s) => {
            format!(
                "{base}{}  # {}",
                s,
                code.local_names
                    .get(s as usize)
                    .cloned()
                    .unwrap_or_default()
            )
        }
        Instr::StoreFast(s) => {
            format!(
                "{base}{}  # {}",
                s,
                code.local_names
                    .get(s as usize)
                    .cloned()
                    .unwrap_or_default()
            )
        }
        Instr::LoadGlobal(n) | Instr::StoreGlobal(n) => {
            format!(
                "{base}{}  # {}",
                n,
                code.names.get(n as usize).cloned().unwrap_or_default()
            )
        }
        Instr::LoadAttr { idx, site } => {
            let name = code.names.get(idx as usize).cloned().unwrap_or_default();
            format!("{base}{idx}  # {name} site={site}")
        }
        Instr::CallFunction { argc, site } => format!("{base}{argc}  site={site}"),
        Instr::CallFunctionKw {
            argc,
            kw_first,
            n_kw,
            site,
        } => format!("{base}{argc}+{n_kw}kw  site={site} kw@{kw_first}"),
        Instr::StoreAttr { idx } => {
            let name = code.names.get(idx as usize).cloned().unwrap_or_default();
            format!("{base}{idx}  # {name}")
        }
        Instr::Jump(t)
        | Instr::JumpIfFalse(t)
        | Instr::JumpIfTrue(t)
        | Instr::JumpIfFalseOrPop(t)
        | Instr::JumpIfTrueOrPop(t)
        | Instr::ForNext(t)
        | Instr::CheckExcept(t) => format!("{base}->{t}"),
        Instr::BinOp(op) => format!("{base}{}", op.as_str()),
        Instr::UnaryOp(op) => format!(
            "{base}{}",
            match op {
                crate::instr::UnaryKind::Plus => "+",
                crate::instr::UnaryKind::Minus => "-",
                crate::instr::UnaryKind::Invert => "~",
                crate::instr::UnaryKind::Not => "not",
            }
        ),
        Instr::Compare(op) => format!(
            "{base}{}",
            match op {
                crate::instr::CompareOp::Eq => "==",
                crate::instr::CompareOp::NotEq => "!=",
                crate::instr::CompareOp::Lt => "<",
                crate::instr::CompareOp::LtE => "<=",
                crate::instr::CompareOp::Gt => ">",
                crate::instr::CompareOp::GtE => ">=",
                crate::instr::CompareOp::Is => "is",
                crate::instr::CompareOp::IsNot => "is not",
                crate::instr::CompareOp::In => "in",
                crate::instr::CompareOp::NotIn => "not in",
            }
        ),
        Instr::BuildTuple(c) | Instr::BuildList(c) | Instr::BuildDict(c) | Instr::BuildSet(c) => {
            format!("{base}{c}")
        }
        _ => base.trim_end().to_string(),
    }
}

/// Disassemble the whole code object.
pub fn disassemble(code: &CodeObject) -> String {
    if code.instrs.len() == 1 && code.instrs[0] == Instr::FallbackToTreeWalk {
        return format!(
            "{}: <FallbackToTreeWalk (tree-walker owns this function)>",
            code.name
        );
    }
    let mut out = format!(
        "{}: {} instrs, {} locals, {} consts, {} sites\n",
        code.name,
        code.instrs.len(),
        code.n_locals,
        code.consts.len(),
        code.sites.len()
    );
    for i in 0..code.instrs.len() {
        out.push_str(&render_instr(code, i));
        out.push('\n');
    }
    out
}
