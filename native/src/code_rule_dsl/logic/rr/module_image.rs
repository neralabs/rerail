//! A module's image (decision 0246 §1, §6): a rule file lowered alone (`lower::Module`), kept as a program image
//! (decision 0173), built by the same builder. Its program is the image's rules, declarations, facts and `.about`
//! rows; what lowering's whole-program half and the modules that include it read of it besides (its `.violation`s,
//! its `must, may` relations and heads, its kind sets, lattices, helpers, what its relations read, its column types
//! as written) are `.about` relations of their own, `module_*`.
use super::lower::{Module, ModuleParts, ViolationParts};
use crate::code_rule_dsl::logic::front::{List, Value};
use crate::code_rule_dsl::logic::program_facts::Facts;
use crate::code_rule_dsl::logic::program_image::{Builder, Cell, Image, SharedBytes, Version};
use std::collections::BTreeMap;

/// The image of module `m`, written once where it lives (decision 0480).
pub fn encode(m: &Module) -> Result<Image, String> {
    let p = m.parts();
    let v = m.version();
    // Its program as its version holds it (decision 0377), its rules in their order: its `must, may` rules' places
    // and its helpers' counts are by that order.
    let mut b = Builder::base();
    for r in v.rules() {
        b.copy(v, r);
    }
    let program = |r: &str| !r.starts_with("module_");
    b.fields().copy_fields(v, &program);
    let mut about = v.about_kept(&program);
    let s = |t: &str| Value::Str(t.into());
    let n = |k: usize| Value::Int(k as i64);
    let mut put = |rel: &str, row: Vec<Value>| about.entry(rel.to_owned()).or_default().push(List::from(row));
    for (k, v) in p.violations.iter().enumerate() {
        put("module_violation", vec![n(k), s(&v.name), n(v.arity), s(&v.code.0), n(v.code.1), s(&v.code.2), n(v.node as usize), n(v.places as usize), s(&v.message), Value::Int(v.tag.map_or(-1, |t| t as i64))]);
        for (c, (col, ty)) in v.columns.iter().enumerate() {
            put("module_violation_col", vec![n(k), n(c), s(col), s(ty)]);
        }
    }
    for r in &p.three {
        put("module_three", vec![s(r)]);
    }
    for (i, w) in &p.modal {
        put("module_modal", vec![n(*i), s(w)]);
    }
    for r in &p.sets {
        put("module_set", vec![s(r)]);
    }
    for (name, base, lub) in &p.lattices {
        put("module_lattice", vec![s(name), s(base), s(lub)]);
    }
    for (stem, k) in &p.made {
        put("module_made", vec![s(stem), n(*k)]);
    }
    put("module_next", vec![n(p.next)]);
    put("module_stated", vec![n(p.stated)]);
    for (name, stem, k) in &p.once {
        put("module_once", vec![s(name), s(stem), n(*k)]);
    }
    for (r, x) in &p.deps {
        put("module_dep", vec![s(r), s(x)]);
    }
    for (r, types) in &p.written {
        put("module_written", vec![s(r), n(types.len())]);
        for (c, t) in types.iter().enumerate() {
            put("module_written_col", vec![s(r), n(c), s(t)]);
        }
    }
    if let Some(d) = p.docof {
        put("module_docof", vec![n(d as usize)]);
    }
    for v in &p.places {
        put("module_place", vec![s(v)]);
    }
    for d in &p.printed {
        put("module_printed", vec![s(d)]);
    }
    b.fields().set_about(&about);
    b.image()
}

/// The module image `bytes` holds, read where they are (decision 0480).
pub fn decode(bytes: SharedBytes) -> Result<Module, String> {
    let v = Version::open_shared(bytes)?;
    // Its program, rules and header, stays in the image, read as the module's version.
    let mut rows: BTreeMap<String, Vec<Vec<Cell>>> = BTreeMap::new();
    for f in v.about_facts() {
        let name = v.text(f.name());
        if name.starts_with("module_") {
            rows.insert(name.to_owned(), f.rows().map(|r| r.collect()).collect());
        }
    }
    let text = |c: &Cell| match c {
        Cell::Str(a) => Ok(v.text(*a).to_owned()),
        Cell::Int(_) | Cell::Branded(_) => Err("a module image's row: a number where a name is".to_owned()),
    };
    let int = |c: &Cell| match c {
        Cell::Int(i) => Ok(*i),
        Cell::Str(_) | Cell::Branded(_) => Err("a module image's row: a name where a number is".to_owned()),
    };
    let of = |rel: &str| rows.get(rel).map(Vec::as_slice).unwrap_or(&[]);
    let mut p = ModuleParts::default();
    let mut violations: BTreeMap<i64, ViolationParts> = BTreeMap::new();
    for r in of("module_violation") {
        let tag = int(&r[9])?;
        violations.insert(int(&r[0])?, ViolationParts {
            name: text(&r[1])?,
            arity: int(&r[2])? as usize,
            code: (text(&r[3])?, int(&r[4])? as usize, text(&r[5])?),
            node: int(&r[6])? != 0,
            places: int(&r[7])? != 0,
            columns: Vec::new(),
            message: text(&r[8])?,
            tag: (tag >= 0).then_some(tag as usize),
        });
    }
    let mut cols: BTreeMap<(i64, i64), (String, String)> = BTreeMap::new();
    for r in of("module_violation_col") {
        cols.insert((int(&r[0])?, int(&r[1])?), (text(&r[2])?, text(&r[3])?));
    }
    for ((k, _), col) in cols {
        violations.get_mut(&k).ok_or("a module image's violation column of no violation")?.columns.push(col);
    }
    p.violations = violations.into_values().collect();
    p.three = of("module_three").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
    p.modal = of("module_modal").iter().map(|r| Ok((int(&r[0])? as usize, text(&r[1])?))).collect::<Result<_, String>>()?;
    p.sets = of("module_set").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
    p.lattices = of("module_lattice").iter().map(|r| Ok((text(&r[0])?, text(&r[1])?, text(&r[2])?))).collect::<Result<_, String>>()?;
    p.made = of("module_made").iter().map(|r| Ok((text(&r[0])?, int(&r[1])? as usize))).collect::<Result<_, String>>()?;
    p.next = of("module_next").first().map(|r| int(&r[0])).transpose()?.unwrap_or(0) as usize;
    p.once = of("module_once").iter().map(|r| Ok((text(&r[0])?, text(&r[1])?, int(&r[2])? as usize))).collect::<Result<_, String>>()?;
    p.stated = of("module_stated").first().map(|r| int(&r[0])).transpose()?.unwrap_or(0) as usize;
    p.deps = of("module_dep").iter().map(|r| Ok((text(&r[0])?, text(&r[1])?))).collect::<Result<_, String>>()?;
    let mut written: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in of("module_written") {
        written.insert(text(&r[0])?, vec![String::new(); int(&r[1])? as usize]);
    }
    for r in of("module_written_col") {
        let (rel, col) = (text(&r[0])?, int(&r[1])? as usize);
        let slot = written.get_mut(&rel).and_then(|t| t.get_mut(col)).ok_or("a module image's column type of no relation")?;
        *slot = text(&r[2])?;
    }
    p.written = written.into_iter().collect();
    p.docof = of("module_docof").first().map(|r| int(&r[0]).map(|d| d != 0)).transpose()?;
    p.printed = of("module_printed").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
    p.places = of("module_place").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
    Module::of_parts(p, std::sync::Arc::clone(&v)).map_err(|e| e.to_string())
}
