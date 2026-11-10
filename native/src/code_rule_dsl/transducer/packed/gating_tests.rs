//! Gated regions, simdjson's skip-when-absent branches generalized (doc 26 §1.1): which work a
//! rare gate puts behind a branch, and that the outputs do not change.

use super::bitlex_compile::{self, FALLBACK};
use super::block::BlockProgram;
use super::native_cpu_block::{self, State};
use super::native_ir::{self, BlockNative, Pass};

/// A small grammar: its block program, its native form, and its named values.
pub(super) struct Small {
    pub(super) block: BlockProgram,
    pub(super) native: BlockNative,
    values: Vec<bitlex_compile::Named>,
}

pub(super) fn small(text: &str) -> Small {
    let stmts = ::bitlex::syntax::parse(text).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, &[FALLBACK], None).expect("compiles");
    let program = super::rir::assemble(&compiled.text()).expect("assembles");
    let native = native_ir::lower(&program).expect("lowers").block_programs.remove(0);
    Small { block: program.block_programs()[0].clone(), native, values: compiled.values }
}

impl Small {
    fn op(&self, name: &str) -> usize {
        usize::from(self.values.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no `{name}`")).op)
    }
}

/// Each pass's regions, as the whole program's values: the gate, and what it skips (its
/// members, and the vector values only they read).
fn regions(native: &BlockNative) -> Vec<(usize, Vec<usize>)> {
    let passes = native.passes();
    let mut out = Vec::new();
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let plan = State::of(body).plan;
        for r in &plan.regions {
            out.push((origin[r.gate] as usize, r.members.iter().chain(&r.vectors).map(|&m| origin[m] as usize).collect()));
        }
    }
    out
}

/// simdjson's escape scanner, in small: a `\u`'s hex digits checked only where a `\u` is.
pub(super) const ESCAPES: &str = "lexer t
class bs  = '\\\\'
class u   = 'u'
class hex = '0'..'9' 'a'..'f'
class rb  = '}'
mask e  = escaped(bs)
mask ue = e & u
mask h4 = next(hex) & next(hex, 2) & next(hex, 3) & next(hex, 4)
mask hb = next(hex) & next(hex, 2) & next(rb, 3)
escape \"bad\" when ue & ~(h4 | hb)
";

#[test]
fn a_rare_escape_gates_its_check_on_the_vectors() {
    let g = small(ESCAPES);
    let regions = regions(&g.native);
    // The hex checks are byte work on the vectors: behind the gate, they cost nothing in a
    // block without a `\u`.
    let (h4, hb) = (g.op("h4"), g.op("hb"));
    assert!(regions.iter().any(|(_, m)| m.contains(&h4) && m.contains(&hb)), "the hex checks are not gated: {regions:?}");
}

/// `ESCAPES` with eight more checks of `ue`, each read after any region on it.
pub(super) fn many() -> String {
    let mut text = String::from(ESCAPES);
    text.push_str("class z = 'z'\n");
    for k in 1..=8 {
        text.push_str(&format!("mask a{k} = ue & next(hex, {k})\nescape \"a{k}\" when a{k} | (z & next(z, {k}))\n"));
    }
    text
}

/// A region whose values are read after it holds each one, from its end to its reader, in a
/// register or a spill: one with many is not worth its branch, however much it skips.
#[test]
fn a_region_read_by_many_values_after_it_is_not_taken() {
    let g = small(&many());
    let a1 = g.op("a1");
    let regions = regions(&g.native);
    assert!(regions.iter().all(|(_, m)| !m.contains(&a1)), "{regions:?}");
    // The hex check, which leaves its region as one value, still is gated.
    assert!(regions.iter().any(|(_, m)| m.contains(&g.op("h4"))), "{regions:?}");
}

/// Files with an escape at every offset across a block edge: every form, and at the end.
fn escape_files() -> Vec<Vec<u8>> {
    let forms: [&[u8]; 6] = [b"\\u12ab", b"\\u12", b"\\uab}", b"\\\\u12ab", b"\\x", b"u12ab"];
    let mut files = Vec::new();
    for t in forms {
        for at in 50..140 {
            let mut f = vec![b'z'; 200];
            f[at..at + t.len()].copy_from_slice(t);
            files.push(f[..at + t.len()].to_vec());
            files.push(f);
        }
    }
    files
}

#[test]
fn a_gated_region_equals_the_interpreter() {
    for text in [ESCAPES.to_string(), many()] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        assert!(compiled.passes.iter().any(|p| p.gate_branches > 0), "no branch for the region");
        for f in &escape_files() {
            assert_eq!(compiled.run_files(&[f]), g.block.run_files(&[f]), "{:?}", String::from_utf8_lossy(f));
        }
    }
}

/// `.rr`'s `\u` check (`h4`, the `\u{…}` forms, `u_ok`) is behind the gate of a `\u` in a
/// string: set in 0.02% of the corpus's blocks.
#[test]
fn rr_gates_its_unicode_escape_check() {
    let c = super::rr_parser::compiled().expect("compiles");
    let regions = regions(&c.native);
    let op = |name: &str| usize::from(c.values.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no `{name}`")).op);
    for name in ["h4", "b_short", "b_four", "b_five", "b_six", "u_ok"] {
        assert!(regions.iter().any(|(_, m)| m.contains(&op(name))), "`{name}` is not gated: {} regions", regions.len());
    }
}

/// The gate after a `back_fill`, as `.rr`'s `uesc` is (a string's fill comes after one): the
/// hex check, made from the bytes alone, must not be made in the first pass and stored.
pub(super) const LATE: &str = "lexer t
class bs  = '\\\\'
class u   = 'u'
class hex = '0'..'9' 'a'..'f'
class sp  = ' '
class z   = 'z'
mask ahead = back_fill(z, ~sp)
mask e  = escaped(bs)
mask ue = e & u & ahead
mask h4 = next(hex) & next(hex, 2) & next(hex, 3) & next(hex, 4)
escape \"bad\" when ue & ~h4
";

/// Each forward pass's values as the whole program's, and the ones it loads.
fn placed(native: &BlockNative) -> Vec<(Vec<usize>, Vec<usize>)> {
    let passes = native.passes();
    let mut out = Vec::new();
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let (mut made, mut loaded) = (Vec::new(), Vec::new());
        for (x, inst) in body.insts.iter().enumerate() {
            let o = origin[x] as usize;
            if matches!(inst, native_ir::BlockInst::Load(_)) { loaded.push(o) } else { made.push(o) }
        }
        out.push((made, loaded));
    }
    out
}

#[test]
fn a_byte_value_is_made_in_the_pass_that_reads_it() {
    let g = small(LATE);
    let (h4, ue) = (g.op("h4"), g.op("ue"));
    let placed = placed(&g.native);
    assert!(placed.iter().all(|(_, loaded)| !loaded.contains(&h4)), "`h4` is stored and loaded: {placed:?}");
    assert!(placed.iter().any(|(made, _)| made.contains(&h4) && made.contains(&ue)), "`h4` is not made with its gate: {placed:?}");
    // And so it is gated there.
    assert!(regions(&g.native).iter().any(|(_, m)| m.contains(&h4)), "`h4` is not gated");
}

#[test]
fn a_byte_value_read_in_two_passes_is_made_in_both() {
    // `hz`'s fill reads `h4` in the first pass; `ue & ~h4` reads it in the last.
    let g = small(&format!("{LATE}mask hz = fill(h4 & z, sp)\nescape \"early\" when hz\n"));
    let h4 = g.op("h4");
    let placed = placed(&g.native);
    assert!(placed.iter().filter(|(made, _)| made.contains(&h4)).count() == 2, "{placed:?}");
    assert!(placed.iter().all(|(_, loaded)| !loaded.contains(&h4)), "{placed:?}");
}

#[test]
fn an_ungated_byte_value_stays_stored() {
    // `h3` is read in the last pass only by `ahead | h3`, which no gate makes zero: remade there
    // it would be held across the body, so it is made in the first pass and loaded.
    let g = small(&format!("{LATE}mask h3 = next(hex) & next(hex, 2) & next(hex, 3) & next(sp, 4)\nescape \"ungated\" when ahead | h3\n"));
    let h3 = g.op("h3");
    let placed = placed(&g.native);
    assert!(placed.iter().filter(|(made, _)| made.contains(&h3)).count() == 1, "{placed:?}");
    assert!(placed.last().is_some_and(|(_, loaded)| loaded.contains(&h3)), "{placed:?}");
}

/// The escape files with spaces and `z`s, so that `ahead` is set and not.
fn late_files() -> Vec<Vec<u8>> {
    let mut files = escape_files();
    for (k, f) in files.iter_mut().enumerate() {
        for (j, b) in f.iter_mut().enumerate() {
            if *b == b'z' && (j * 7 + k) % 5 == 0 {
                *b = b' ';
            }
        }
    }
    files
}

#[test]
fn remade_byte_values_equal_the_interpreter() {
    for text in [
        LATE.to_string(),
        format!("{LATE}mask hz = fill(h4 & z, sp)\nescape \"early\" when hz\n"),
        format!("{LATE}mask h3 = next(hex) & next(hex, 2) & next(hex, 3) & next(sp, 4)\nescape \"ungated\" when ahead | h3\n"),
    ] {
        let g = small(&text);
        let compiled = native_cpu_block::compile(&g.native).expect("compiles");
        for f in &late_files() {
            assert_eq!(compiled.run_files(&[f]), g.block.run_files(&[f]), "{:?}", String::from_utf8_lossy(f));
        }
    }
}

#[test]
#[ignore = "a dump: run by hand"]
fn dump_small_plan() {
    let text = std::env::var("GRAMMAR").map(|p| std::fs::read_to_string(p).expect("a grammar file")).unwrap_or_else(|_| if std::env::var("LATE").is_ok() { LATE } else { ESCAPES }.to_string());
    let g = small(&text);
    let pure = g.native.pure();
    let trial = g.native.passes_with(&pure);
    for (k, p) in trial.passes.iter().enumerate() {
        if let Pass::Forward { body, .. } = p {
            for r in &State::of(body).plan.regions {
                let o = |x: &usize| trial.origins[k][*x];
                eprintln!("trial p{k} gate <{}> members {:?} vectors {:?}", o(&r.gate), r.members.iter().map(o).collect::<Vec<_>>(), r.vectors.iter().map(o).collect::<Vec<_>>());
            }
        }
    }
    let remade = super::native_cpu_lanes::remade(&g.native);
    eprintln!("remade {:?}", (0..remade.len()).filter(|&x| remade[x]).collect::<Vec<_>>());
    for v in &g.values {
        eprintln!("name {} = <{}>", v.name, v.op);
    }
    let passes = g.native.passes();
    for (k, p) in passes.passes.iter().enumerate() {
        if let Pass::Forward { body, .. } = p {
            let plan = State::of(body).plan;
            for (x, inst) in body.insts.iter().enumerate() {
                eprintln!("p{k} %{x:<3} <{}> {inst:?} lane {} mask {} full {:?}", passes.origins[k][x], plan.lanes[x], plan.mask[x], plan.full[x]);
            }
            eprintln!("p{k} outputs {:?} regions {}", body.outputs, plan.regions.len());
        }
    }
    let plan = State::of(&g.native).plan;
    for (x, inst) in g.native.insts.iter().enumerate() {
        let name: Vec<&str> = g.values.iter().filter(|v| usize::from(v.op) == x).map(|v| v.name.as_str()).collect();
        eprintln!("%{x:<3} {inst:<40?} lane {} mask {} full {:?} {name:?}", plan.lanes[x], plan.mask[x], plan.full[x]);
    }
    eprintln!("outputs {:?}", g.native.outputs);
}

#[test]
#[ignore = "a dump: run by hand"]
fn dump_rr_unicode() {
    let c = super::rr_parser::compiled().expect("compiles");
    let passes = c.native.passes_with(&c.native.pure());
    let names = ["uesc", "h4", "u_four", "u_brace", "b_short", "b_four", "b_six", "u_ok"];
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let plan = State::of(body).plan;
        let local = |name: &str| c.values.iter().find(|v| v.name == name).and_then(|v| origin.iter().position(|&o| o as usize == usize::from(v.op)));
        for name in names {
            let Some(x) = local(name) else { continue };
            let region: Vec<usize> = plan.regions.iter().enumerate().filter(|(_, r)| r.members.contains(&x) || r.vectors.contains(&x)).map(|(i, _)| i).collect();
            let users: Vec<String> = body.insts.iter().enumerate().filter(|(_, i)| i.operands().contains(&x)).map(|(u, i)| format!("%{u} {i:?} lane {} mask {} region {:?}", plan.lanes[u], plan.mask[u], plan.region_of[u])).collect();
            eprintln!("pass {k} `{name}` %{x} {:?} lane {} mask {} regions {region:?} users {users:?}", body.insts[x], plan.lanes[x], plan.mask[x]);
        }
        for (i, r) in plan.regions.iter().enumerate() {
            eprintln!("pass {k} region {i}: gate %{} {:?}, {} members, {} vectors", r.gate, body.insts[r.gate], r.members.len(), r.vectors.len());
        }
    }
}
