//! Tier 2 on the CPU, generated (docs/parser-generators/17-tier-2-tdd-plan.md §5): Cranelift
//! emits one function per block program from its rows, as tier 1 is emitted from its operations.
//! It reads the masks tier 1 wrote, as simdjson's stage 2 reads stage 1's indexes, and writes the
//! nodes in preorder. Nothing reads the rows at run time: each row is code.
//!
//! The function walks the events (every bit of a row's start or end mask) in byte order. At each:
//!
//! - **the token** that starts there picks one case of a `br_table`, an arm per token row and one
//!   for a byte where no token starts. In its arm the token's kind is known (or read from its
//!   keywords, inlined as compares), and the rows' starts are straight-line tests in rank order,
//!   folded where the case decides them;
//! - **the ends** close the innermost open node while its row ends there, by a `br_table` on the
//!   row of the stack's top; a node that ends `with` its child's field closes with it;
//! - **the checks** send the file to tree-sitter: two tokens at a byte, bytes in no token, a stray
//!   or too-deep close, a token's validator, a second root.
//!
//! The CPU plan (`block_plan::plan`, rule `also`) is one pass: a row found `before` a token (from
//! the right) is opened by its `also` rule (from the left), and the `before` rule is checked at
//! the token after its first child. A file where the two differ goes to tree-sitter.
//!
//! The columns are packed by the tape's one packer (`TapeImage::from_written`).

use super::block::{BlockProgram, Fact, RowEnd, RowStart, Validator};
use super::block_plan::{Rule, Target, plan};
use crate::code_rule_dsl::tape::{Grammar, TapeImage, WrittenNode};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, Block, BlockArg, InstBuilder, JumpTableData, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// A node as the function writes it: 16 bytes. `size`'s top bit is the extra flag.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Node {
    kind: u16,
    field: u16,
    start: u32,
    end: u32,
    size: u32,
}

/// The masks (word-major: word `w` of output `k` at `w * outputs + k`) and their words per
/// output; the source (8 bytes of padding after it) and its length; the nodes, the supertype
/// memberships (`ord | class << 32`), the stack (a sentinel slot, then one word per open node:
/// `ord | row << 32 | kind << 48`), and the two counts. Returns 0, or 1 for a file that goes
/// to tree-sitter.
type Entry = unsafe extern "C" fn(*const u64, u64, *const u8, u64, *mut Node, *mut u64, *mut u64, *mut u64) -> u64;

/// No token yet: a kind no grammar has.
const NONE: i64 = 0xffff;

/// A compiled tier 2.
pub(super) struct CompiledTier2 {
    _module: JITModule,
    entry: Entry,
    outputs: usize,
    /// Each output whose bits are node starts, and how many nodes each bit may start; and how
    /// many each token start may (`Kind` rows), the first token's rows, and the most supertypes
    /// a node may have: the bounds of the columns.
    start_outputs: Vec<(usize, usize)>,
    leaf_outputs: Vec<usize>,
    per_token: usize,
    first_rows: usize,
    max_classes: usize,
    /// The function as Cranelift IR, before code generation.
    pub(super) clif: String,
}

/// A validator's entry: the token's text, its length; nonzero when valid.
extern "C" fn json_number(text: *const u8, len: u64) -> u64 {
    // SAFETY: the function passes a token of the source, which lies in its buffer.
    let t = unsafe { std::slice::from_raw_parts(text, len as usize) };
    u64::from(valid(Validator::JsonNumber, t))
}

fn validator(v: Validator) -> usize {
    match v {
        Validator::JsonNumber => json_number as *const () as usize,
    }
}

impl CompiledTier2 {
    /// The nodes of a file in preorder and its supertype memberships, from tier 1's `outputs`
    /// over `source`; `None` when the file goes to tree-sitter.
    pub(super) fn write(&self, outputs: &[Vec<u64>], source: &[u8]) -> Option<(Vec<WrittenNode>, Vec<(u32, u16)>)> {
        let len = source.len();
        let words = len.div_ceil(64);
        let mut masks = vec![0u64; words * self.outputs];
        for (k, out) in outputs.iter().enumerate().take(self.outputs) {
            for w in 0..words {
                masks[w * self.outputs + k] = out.get(w).copied().unwrap_or(0);
            }
            if len % 64 != 0 {
                masks[(words - 1) * self.outputs + k] &= (1u64 << (len % 64)) - 1;
            }
        }
        let count = |k: usize| (0..words).map(|w| masks[w * self.outputs + k].count_ones() as usize).sum::<usize>();
        let tokens: usize = self.leaf_outputs.iter().map(|&k| count(k)).sum();
        let bound = self.first_rows + tokens * self.per_token + self.start_outputs.iter().map(|&(k, n)| count(k) * n).sum::<usize>();
        let mut nodes = vec![Node::default(); bound];
        let mut supers = vec![0u64; bound * self.max_classes];
        let mut stack = vec![u64::MAX; bound + 1];
        let mut counts = [0u64; 2];
        let mut padded = Vec::with_capacity(len + 8);
        padded.extend_from_slice(source);
        padded.resize(len + 8, 0);
        // SAFETY: `masks` holds `words` words of every output; the source is padded by 8 bytes;
        // the nodes, supertypes and stack hold the most the masks can start (`bound`).
        let failed = unsafe {
            (self.entry)(masks.as_ptr(), words as u64, padded.as_ptr(), len as u64, nodes.as_mut_ptr(), supers.as_mut_ptr(), stack.as_mut_ptr(), counts.as_mut_ptr())
        };
        if failed != 0 {
            return None;
        }
        let written = nodes[..counts[0] as usize]
            .iter()
            .map(|n| WrittenNode { kind: n.kind, field: n.field, extra: n.size >> 31 == 1, start: n.start, end: n.end, size: n.size & 0x7fff_ffff })
            .collect();
        let classes = supers[..counts[1] as usize].iter().map(|&s| (s as u32, (s >> 32) as u16)).collect();
        Some((written, classes))
    }

    /// The tape of `source` from tier 1's outputs: `None` when the file goes to tree-sitter.
    pub(super) fn tape(&self, outputs: &[Vec<u64>], grammar: &Grammar, source: &str) -> Result<Option<TapeImage>, String> {
        let Some((nodes, supertypes)) = self.write(outputs, source.as_bytes()) else {
            return Ok(None);
        };
        TapeImage::from_written(&nodes, supertypes, grammar, source, &|| false).map(Some)
    }
}

/// The function's variables.
struct Vars {
    n: Variable,
    ns: Variable,
    sp: Variable,
    depth: Variable,
    first: Variable,
    prev: Variable,
    leaf_end: Variable,
    w: Variable,
    wp: Variable,
    events: Variable,
    at: Variable,
    bit: Variable,
    kind: Variable,
    here: Variable,
    closed: Variable,
    ended: Variable,
    top: Variable,
    at_close: Variable,
    last_field: Variable,
    mode: Variable,
    field: Variable,
    /// Per row checked `before`: a start waiting for its first child to close (`pend`, the
    /// child's ord, whether the node opened), then waiting for the next token (`wait`).
    pend: Vec<(usize, Variable, Variable, Variable)>,
    wait: Vec<(Variable, Variable)>,
}

/// A row's start condition on the CPU: the forward parts of the rule it opens by.
struct Opens {
    after: Vec<u16>,
    parent: Option<u16>,
}

/// Tier 2 of `p` on the CPU. `fallbacks`: tier 1's outputs whose bits send the file to
/// tree-sitter.
pub(super) fn compile(p: &BlockProgram, fallbacks: &[u16]) -> Result<CompiledTier2, String> {
    let cpu = plan(p, Target::Cpu)?;
    if cpu.rule != Rule::Also && p.nodes.iter().any(|r| r.also.is_some()) {
        return Err("the CPU plan opens rows by their also rule".into());
    }
    let rows = &p.nodes;
    if rows.len() > 64 {
        return Err(format!("{} rows: at most 64", rows.len()));
    }
    let out = |v: u16| -> Result<usize, String> { p.outputs.iter().position(|&o| o == v).ok_or_else(|| format!("%{v}, a row's mask, is not an output")) };
    let start_out: Vec<Option<usize>> = rows.iter().map(|r| if let RowStart::At(v) = r.start { out(v).map(Some) } else { Ok(None) }).collect::<Result<_, _>>()?;
    let end_out: Vec<Option<usize>> = rows.iter().map(|r| if let RowEnd::At(v) = r.end { out(v).map(Some) } else { Ok(None) }).collect::<Result<_, _>>()?;
    let skip = p.skip.map(out).transpose()?;
    let leaves: Vec<usize> = (0..rows.len()).filter(|&r| rows[r].leaf).collect();
    for &r in &leaves {
        if start_out[r].is_none() {
            return Err(format!("token row {r} starts at no mask"));
        }
    }
    // The forward rule each row opens by, and the `before` each checks.
    let mut opens = Vec::new();
    let mut checks: Vec<(usize, Vec<u16>)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        match &row.also {
            Some(also) => {
                if !also.before.is_empty() {
                    return Err(format!("row {r}: an also rule read from the right"));
                }
                let mut after = also.after.clone();
                if !row.after.is_empty() {
                    after.retain(|k| row.after.contains(k));
                }
                opens.push(Opens { after, parent: also.parent.or(row.parent) });
                if !row.before.is_empty() {
                    checks.push((r, row.before.clone()));
                }
            }
            None if !row.before.is_empty() => return Err(format!("row {r}: before, with no also rule for the CPU")),
            None => opens.push(Opens { after: row.after.clone(), parent: row.parent }),
        }
    }
    let mut keywords: Vec<Vec<(&[u8], u16)>> = vec![Vec::new(); rows.len()];
    for k in &p.keywords {
        keywords[usize::from(k.node)].push((&k.word[..], k.kind));
    }
    for &r in &leaves {
        if !keywords[r].is_empty() && end_out[r].is_none() {
            return Err(format!("token row {r} has keywords and no end mask"));
        }
    }
    let fact_out = |f: &Fact| match f {
        Fact::At(v) => out(*v).map(Some),
        _ => Ok(None),
    };
    for c in &p.codes {
        fact_out(&c.when)?;
    }
    let fallbacks: Vec<usize> = fallbacks.iter().map(|&k| usize::from(k)).collect();
    let outputs = p.outputs.len();
    let stride = (outputs * 8) as i64;

    let builder = JITBuilder::new(default_libcall_names()).map_err(|e| format!("jit: {e}"))?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    for _ in 0..8 {
        signature.params.push(AbiParam::new(types::I64));
    }
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("tempo_tier2", Linkage::Local, &signature).map_err(|e| format!("declare: {e}"))?;
    let mut validator_sig = module.make_signature();
    validator_sig.params.push(AbiParam::new(pointer));
    validator_sig.params.push(AbiParam::new(types::I64));
    validator_sig.returns.push(AbiParam::new(types::I64));
    let mut context: Context = module.make_context();
    context.func.signature = signature;
    let mut fbc = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut fbc);
        let vsig = f.import_signature(validator_sig);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let params = f.block_params(entry).to_vec();
        let (masks, words, src, len, nodes, supers, stack, counts) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6], params[7]);
        let i64v = |f: &mut FunctionBuilder<'_>| f.declare_var(types::I64);
        let mut vars = Vars {
            n: i64v(&mut f),
            ns: i64v(&mut f),
            sp: i64v(&mut f),
            depth: i64v(&mut f),
            first: i64v(&mut f),
            prev: i64v(&mut f),
            leaf_end: i64v(&mut f),
            w: i64v(&mut f),
            wp: i64v(&mut f),
            events: i64v(&mut f),
            at: i64v(&mut f),
            bit: i64v(&mut f),
            kind: i64v(&mut f),
            here: i64v(&mut f),
            closed: i64v(&mut f),
            ended: i64v(&mut f),
            top: i64v(&mut f),
            at_close: i64v(&mut f),
            last_field: i64v(&mut f),
            mode: i64v(&mut f),
            field: i64v(&mut f),
            pend: Vec::new(),
            wait: Vec::new(),
        };
        for &(r, _) in &checks {
            let (a, b, c) = (i64v(&mut f), i64v(&mut f), i64v(&mut f));
            vars.pend.push((r, a, b, c));
            let (d, e) = (i64v(&mut f), i64v(&mut f));
            vars.wait.push((d, e));
        }
        let zero = f.ins().iconst(types::I64, 0);
        for v in [vars.n, vars.ns, vars.sp, vars.depth, vars.first, vars.leaf_end, vars.w, vars.wp, vars.events, vars.at, vars.bit, vars.kind, vars.here, vars.closed, vars.top, vars.at_close, vars.last_field, vars.mode, vars.field] {
            f.def_var(v, zero);
        }
        let none = f.ins().iconst(types::I64, NONE);
        f.def_var(vars.prev, none);
        f.def_var(vars.ended, none);
        for &(_, a, b, c) in &vars.pend {
            f.def_var(a, zero);
            f.def_var(b, zero);
            f.def_var(c, zero);
        }
        for &(d, e) in &vars.wait {
            f.def_var(d, zero);
            f.def_var(e, zero);
        }
        let stack_base = f.ins().iadd_imm(stack, 8);

        let fail = f.create_block();
        let word_head = f.create_block();
        let ev_head = f.create_block();
        let end_head = f.create_block();
        let with_head = f.create_block();
        let ends_done = f.create_block();
        let eof = f.create_block();
        let eof_head = f.create_block();
        let finish = f.create_block();
        f.ins().jump(word_head, &[]);

        let flags = MemFlags::trusted();
        // Helpers over the builder.
        let word_of = |f: &mut FunctionBuilder<'_>, vars: &Vars, k: usize| {
            let wp = f.use_var(vars.wp);
            f.ins().load(types::I64, flags, wp, (8 * k) as i32)
        };
        let bit_of = |f: &mut FunctionBuilder<'_>, vars: &Vars, k: usize| {
            let m = word_of(f, vars, k);
            let bit = f.use_var(vars.bit);
            let x = f.ins().band(m, bit);
            let b = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
            f.ins().uextend(types::I64, b)
        };
        let any_of = |f: &mut FunctionBuilder<'_>, x: Value, set: &[u16]| {
            let mut acc = f.ins().iconst(types::I64, 0);
            for &k in set {
                let e = f.ins().icmp_imm(IntCC::Equal, x, i64::from(k));
                let e = f.ins().uextend(types::I64, e);
                acc = f.ins().bor(acc, e);
            }
            acc
        };
        // Continue at `then` if `c` (an i64, 0 or 1) is nonzero, else at `otherwise`.
        let branch = |f: &mut FunctionBuilder<'_>, c: Value, then: Block, otherwise: Block| {
            f.ins().brif(c, then, &[], otherwise, &[]);
        };
        // Fall to a new block if `c`, else fail.
        let ensure = |f: &mut FunctionBuilder<'_>, c: Value| {
            let ok = f.create_block();
            f.ins().brif(c, ok, &[], fail, &[]);
            f.switch_to_block(ok);
        };
        // The bytes `from..to` are all skipped, or fail.
        let skipped = |f: &mut FunctionBuilder<'_>, from: Value, to: Value| {
            let done = f.create_block();
            let lp = f.create_block();
            f.append_block_param(lp, types::I64);
            let empty = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, from, to);
            let first = f.ins().ushr_imm(from, 6);
            f.ins().brif(empty, done, &[], lp, &[BlockArg::Value(first)]);
            f.switch_to_block(lp);
            let ww = f.block_params(lp)[0];
            match skip {
                None => {
                    f.ins().jump(fail, &[]);
                }
                Some(k) => {
                    let to1 = f.ins().iadd_imm(to, -1);
                    let last = f.ins().ushr_imm(to1, 6);
                    let at = f.ins().imul_imm(ww, stride);
                    let at = f.ins().iadd(masks, at);
                    let m = f.ins().load(types::I64, flags, at, (8 * k) as i32);
                    let lo_from = f.ins().band_imm(from, 63);
                    let is_first = f.ins().icmp(IntCC::Equal, ww, first);
                    let lo = f.ins().select(is_first, lo_from, zero);
                    let hi_to = f.ins().band_imm(to1, 63);
                    let full = f.ins().iconst(types::I64, 63);
                    let is_last = f.ins().icmp(IntCC::Equal, ww, last);
                    let hi = f.ins().select(is_last, hi_to, full);
                    let ones = f.ins().iconst(types::I64, -1);
                    let a = f.ins().ishl(ones, lo);
                    let sh = f.ins().isub(full, hi);
                    let b = f.ins().ushr(ones, sh);
                    let need = f.ins().band(a, b);
                    let have = f.ins().band(m, need);
                    let ok = f.ins().icmp(IntCC::Equal, have, need);
                    let ok = f.ins().uextend(types::I64, ok);
                    ensure(f, ok);
                    let next = f.ins().iadd_imm(ww, 1);
                    let more = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, next, last);
                    f.ins().brif(more, lp, &[BlockArg::Value(next)], done, &[]);
                }
            }
            f.switch_to_block(done);
        };
        // The stack's top (the sentinel when empty).
        let top_of = |f: &mut FunctionBuilder<'_>, vars: &Vars| {
            let sp = f.use_var(vars.sp);
            let off = f.ins().ishl_imm(sp, 3);
            let a = f.ins().iadd(stack_base, off);
            f.ins().load(types::I64, flags, a, -8)
        };
        // A `br_table` on `index` (an i64): `arms[i]` for `i`, else `default`.
        let table = |f: &mut FunctionBuilder<'_>, index: Value, default: Block, arms: &[Block]| {
            let def = f.func.dfg.block_call(default, &[]);
            let calls: Vec<_> = arms.iter().map(|&b| f.func.dfg.block_call(b, &[])).collect();
            let jt = f.create_jump_table(JumpTableData::new(def, &calls));
            let i = f.ins().ireduce(types::I32, index);
            f.ins().br_table(i, jt);
        };

        // The word's head: the fallbacks, then the events.
        f.switch_to_block(word_head);
        let w = f.use_var(vars.w);
        let more = f.ins().icmp(IntCC::UnsignedLessThan, w, words);
        let word_body = f.create_block();
        f.ins().brif(more, word_body, &[], eof, &[]);
        f.switch_to_block(word_body);
        let off = f.ins().imul_imm(w, stride);
        let wp = f.ins().iadd(masks, off);
        f.def_var(vars.wp, wp);
        if !fallbacks.is_empty() {
            let mut any = zero;
            for &k in &fallbacks {
                let m = word_of(&mut f, &vars, k);
                any = f.ins().bor(any, m);
            }
            let clean = f.ins().icmp_imm(IntCC::Equal, any, 0);
            let clean = f.ins().uextend(types::I64, clean);
            ensure(&mut f, clean);
        }
        let mut event_outs: Vec<usize> = start_out.iter().chain(&end_out).flatten().copied().collect();
        event_outs.sort_unstable();
        event_outs.dedup();
        let mut events = zero;
        for &k in &event_outs {
            let m = word_of(&mut f, &vars, k);
            events = f.ins().bor(events, m);
        }
        f.def_var(vars.events, events);
        f.ins().jump(ev_head, &[]);

        // An event: the token that starts here picks a case.
        f.switch_to_block(ev_head);
        let events = f.use_var(vars.events);
        let ev_body = f.create_block();
        let next_word = f.create_block();
        f.ins().brif(events, ev_body, &[], next_word, &[]);
        f.switch_to_block(next_word);
        let w = f.use_var(vars.w);
        let w1 = f.ins().iadd_imm(w, 1);
        f.def_var(vars.w, w1);
        f.ins().jump(word_head, &[]);
        f.switch_to_block(ev_body);
        let j = f.ins().ctz(events);
        let e1 = f.ins().iadd_imm(events, -1);
        let rest = f.ins().band(events, e1);
        f.def_var(vars.events, rest);
        let w = f.use_var(vars.w);
        let base = f.ins().ishl_imm(w, 6);
        let at = f.ins().iadd(base, j);
        f.def_var(vars.at, at);
        let one = f.ins().iconst(types::I64, 1);
        let bit = f.ins().ishl(one, j);
        f.def_var(vars.bit, bit);
        let mut count = zero;
        let mut index = f.ins().iconst(types::I64, leaves.len() as i64);
        for (i, &r) in leaves.iter().enumerate() {
            let t = bit_of(&mut f, &vars, start_out[r].expect("checked"));
            count = f.ins().iadd(count, t);
            let ii = f.ins().iconst(types::I64, i as i64);
            index = f.ins().select(t, ii, index);
        }
        let single = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, count, 1);
        let single = f.ins().uextend(types::I64, single);
        ensure(&mut f, single);
        let cases: Vec<Block> = leaves.iter().map(|_| f.create_block()).collect();
        let no_token = f.create_block();
        table(&mut f, index, no_token, &cases);

        let first_rows: Vec<usize> = (0..rows.len()).filter(|&r| rows[r].start == RowStart::FirstToken).collect();
        for case in 0..=leaves.len() {
            let token = leaves.get(case).copied();
            f.switch_to_block(if let Some(_) = token { cases[case] } else { no_token });
            // The token's kind: its row's, or its keyword's by its text.
            if let Some(c) = token {
                let row = &rows[c];
                if keywords[c].is_empty() {
                    let k = f.ins().iconst(types::I64, i64::from(row.kind));
                    f.def_var(vars.kind, k);
                } else {
                    // The token's last byte: the first bit of its end mask at or after it.
                    let e = end_out[c].expect("checked");
                    let found = f.create_block();
                    let lp = f.create_block();
                    f.append_block_param(lp, types::I64);
                    f.append_block_param(lp, types::I64);
                    f.append_block_param(found, types::I64);
                    let at = f.use_var(vars.at);
                    let ww = f.ins().ushr_imm(at, 6);
                    let lo = f.ins().band_imm(at, 63);
                    let ones = f.ins().iconst(types::I64, -1);
                    let from = f.ins().ishl(ones, lo);
                    let m = word_of(&mut f, &vars, e);
                    let bits = f.ins().band(m, from);
                    f.ins().jump(lp, &[BlockArg::Value(ww), BlockArg::Value(bits)]);
                    f.switch_to_block(lp);
                    let (ww, bits) = (f.block_params(lp)[0], f.block_params(lp)[1]);
                    let step = f.create_block();
                    let tz = f.ins().ctz(bits);
                    let wb = f.ins().ishl_imm(ww, 6);
                    let last = f.ins().iadd(wb, tz);
                    f.ins().brif(bits, found, &[BlockArg::Value(last)], step, &[]);
                    f.switch_to_block(step);
                    let ww1 = f.ins().iadd_imm(ww, 1);
                    let inside = f.ins().icmp(IntCC::UnsignedLessThan, ww1, words);
                    let inside = f.ins().uextend(types::I64, inside);
                    ensure(&mut f, inside);
                    let off = f.ins().imul_imm(ww1, stride);
                    let a = f.ins().iadd(masks, off);
                    let m = f.ins().load(types::I64, flags, a, (8 * e) as i32);
                    f.ins().jump(lp, &[BlockArg::Value(ww1), BlockArg::Value(m)]);
                    f.switch_to_block(found);
                    let last = f.block_params(found)[0];
                    let at = f.use_var(vars.at);
                    let length = f.ins().isub(last, at);
                    let length = f.ins().iadd_imm(length, 1);
                    let kind_done = f.create_block();
                    let mut lengths: Vec<usize> = keywords[c].iter().map(|k| k.0.len()).collect();
                    lengths.sort_unstable();
                    lengths.dedup();
                    let text = f.ins().iadd(src, at);
                    for l in lengths {
                        let this = f.create_block();
                        let other = f.create_block();
                        let is = f.ins().icmp_imm(IntCC::Equal, length, l as i64);
                        f.ins().brif(is, this, &[], other, &[]);
                        f.switch_to_block(this);
                        // The text's words, read once for every keyword of this length.
                        let chunks: Vec<Value> = (0..l.div_ceil(8))
                            .map(|i| {
                                let x = f.ins().load(types::I64, MemFlags::new().with_notrap(), text, (8 * i) as i32);
                                let rem = l - 8 * i;
                                if rem < 8 { f.ins().band_imm(x, ((1u64 << (8 * rem)) - 1) as i64) } else { x }
                            })
                            .collect();
                        for &(word, kind) in keywords[c].iter().filter(|k| k.0.len() == l) {
                            let mut same = f.ins().iconst(types::I64, 1);
                            for (i, &x) in chunks.iter().enumerate() {
                                let mut b = [0u8; 8];
                                let part = &word[8 * i..word.len().min(8 * i + 8)];
                                b[..part.len()].copy_from_slice(part);
                                let e = f.ins().icmp_imm(IntCC::Equal, x, i64::from_le_bytes(b));
                                let e = f.ins().uextend(types::I64, e);
                                same = f.ins().band(same, e);
                            }
                            let next = f.create_block();
                            let hit = f.create_block();
                            f.ins().brif(same, hit, &[], next, &[]);
                            f.switch_to_block(hit);
                            let k = f.ins().iconst(types::I64, i64::from(kind));
                            f.def_var(vars.kind, k);
                            f.ins().jump(kind_done, &[]);
                            f.switch_to_block(next);
                        }
                        f.ins().jump(other, &[]);
                        f.switch_to_block(other);
                    }
                    // No keyword: the row's kind, or (kind 0) tree-sitter.
                    if row.kind == 0 {
                        f.ins().jump(fail, &[]);
                    } else {
                        let k = f.ins().iconst(types::I64, i64::from(row.kind));
                        f.def_var(vars.kind, k);
                        f.ins().jump(kind_done, &[]);
                    }
                    f.switch_to_block(kind_done);
                }
                // The `before` checks waiting for this token.
                for (i, (_, before)) in checks.iter().enumerate() {
                    let (wait, opened) = vars.wait[i];
                    let kind = f.use_var(vars.kind);
                    let is = any_of(&mut f, kind, before);
                    let opened_v = f.use_var(opened);
                    let differ = f.ins().icmp(IntCC::NotEqual, is, opened_v);
                    let differ = f.ins().uextend(types::I64, differ);
                    let waiting = f.use_var(wait);
                    let bad = f.ins().band(differ, waiting);
                    let ok = f.ins().icmp_imm(IntCC::Equal, bad, 0);
                    let ok = f.ins().uextend(types::I64, ok);
                    ensure(&mut f, ok);
                    f.def_var(wait, zero);
                }
                // The first token opens the rows that start with it, the bytes before it skipped.
                let first = f.use_var(vars.first);
                let opening = f.create_block();
                let opened = f.create_block();
                f.ins().brif(first, opened, &[], opening, &[]);
                f.switch_to_block(opening);
                f.def_var(vars.first, one);
                let at = f.use_var(vars.at);
                skipped(&mut f, zero, at);
                for &r in &first_rows {
                    open_node(&mut f, &vars, p, r, None, nodes, supers, stack_base, fail, &fact_out, &bit_of)?;
                }
                f.ins().jump(opened, &[]);
                f.switch_to_block(opened);
                let leaf_end = f.use_var(vars.leaf_end);
                let at = f.use_var(vars.at);
                skipped(&mut f, leaf_end, at);
            }
            // Starts, outermost first.
            f.def_var(vars.here, zero);
            for (r, row) in rows.iter().enumerate() {
                if row.start == RowStart::FirstToken {
                    continue;
                }
                // Whether the row starts here: decided by the case, or a test.
                let decided: Option<bool> = match row.start {
                    RowStart::At(_) if row.leaf => Some(token == Some(r)),
                    RowStart::At(_) => {
                        let k = start_out[r].expect("an output");
                        if token.is_some_and(|c| start_out[c] == Some(k)) {
                            Some(true)
                        } else if leaves.iter().any(|&l| start_out[l] == Some(k)) {
                            Some(false)
                        } else {
                            None
                        }
                    }
                    RowStart::Kind(k) => match token {
                        None => Some(false),
                        Some(c) if keywords[c].is_empty() => Some(rows[c].kind == k),
                        Some(_) => None,
                    },
                    RowStart::FirstToken => unreachable!(),
                };
                if decided == Some(false) {
                    continue;
                }
                let starts = match (decided, row.start) {
                    (Some(_), _) => one,
                    (None, RowStart::At(_)) => bit_of(&mut f, &vars, start_out[r].expect("an output")),
                    (None, RowStart::Kind(k)) => {
                        let kind = f.use_var(vars.kind);
                        let e = f.ins().icmp_imm(IntCC::Equal, kind, i64::from(k));
                        f.ins().uextend(types::I64, e)
                    }
                    _ => unreachable!(),
                };
                let cond = &opens[r];
                let mut holds = starts;
                if !cond.after.is_empty() {
                    let prev = f.use_var(vars.prev);
                    let a = any_of(&mut f, prev, &cond.after);
                    holds = f.ins().band(holds, a);
                }
                if let Some(parent) = cond.parent {
                    let top = top_of(&mut f, &vars);
                    let kind = f.ins().ushr_imm(top, 48);
                    let e = f.ins().icmp_imm(IntCC::Equal, kind, i64::from(parent));
                    let e = f.ins().uextend(types::I64, e);
                    holds = f.ins().band(holds, e);
                }
                // A row checked `before`: its start waits for its first child, the next node.
                if let Some(i) = vars.pend.iter().position(|&(row, ..)| row == r) {
                    let (_, pend, child, opened) = vars.pend[i];
                    let started = f.create_block();
                    let after = f.create_block();
                    branch(&mut f, starts, started, after);
                    f.switch_to_block(started);
                    let busy = f.use_var(pend);
                    let free = f.ins().icmp_imm(IntCC::Equal, busy, 0);
                    let free = f.ins().uextend(types::I64, free);
                    ensure(&mut f, free);
                    f.def_var(pend, one);
                    let n = f.use_var(vars.n);
                    let c = f.ins().iadd(n, holds);
                    f.def_var(child, c);
                    f.def_var(opened, holds);
                    f.ins().jump(after, &[]);
                    f.switch_to_block(after);
                }
                let then = f.create_block();
                let after = f.create_block();
                branch(&mut f, holds, then, after);
                f.switch_to_block(then);
                let kind = (token == Some(r) && !keywords[r].is_empty()).then_some(vars.kind);
                open_node(&mut f, &vars, p, r, kind, nodes, supers, stack_base, fail, &fact_out, &bit_of)?;
                let here = f.use_var(vars.here);
                let h = f.ins().bor_imm(here, 1i64 << r);
                f.def_var(vars.here, h);
                f.ins().jump(after, &[]);
                f.switch_to_block(after);
            }
            if token.is_some() {
                let kind = f.use_var(vars.kind);
                f.def_var(vars.prev, kind);
            }
            f.def_var(vars.ended, none);
            f.def_var(vars.closed, zero);
            let at = f.use_var(vars.at);
            f.def_var(vars.at_close, at);
            f.def_var(vars.mode, zero);
            f.ins().jump(end_head, &[]);
        }

        // Whether row `r` ends at this event.
        let ends_here = |f: &mut FunctionBuilder<'_>, vars: &Vars, r: usize| -> Option<Value> {
            match rows[r].end {
                RowEnd::At(_) => Some(bit_of(f, vars, end_out[r].expect("an output"))),
                RowEnd::Kind(k) => {
                    let ended = f.use_var(vars.ended);
                    let e = f.ins().icmp_imm(IntCC::Equal, ended, i64::from(k));
                    Some(f.ins().uextend(types::I64, e))
                }
                RowEnd::With(_) | RowEnd::LastToken => None,
            }
        };
        let closers: Vec<Block> = rows.iter().map(|_| f.create_block()).collect();

        // The ends: the stack's top, while its row ends here.
        f.switch_to_block(end_head);
        let sp = f.use_var(vars.sp);
        let has = f.create_block();
        f.ins().brif(sp, has, &[], ends_done, &[]);
        f.switch_to_block(has);
        let top = top_of(&mut f, &vars);
        f.def_var(vars.top, top);
        let row = f.ins().ushr_imm(top, 32);
        let row = f.ins().band_imm(row, 0xffff);
        let tests: Vec<Block> = rows.iter().map(|_| f.create_block()).collect();
        table(&mut f, row, ends_done, &tests);
        for (r, &test) in tests.iter().enumerate() {
            f.switch_to_block(test);
            let Some(ends) = ends_here(&mut f, &vars, r) else {
                f.ins().jump(ends_done, &[]);
                continue;
            };
            let mut ends = ends;
            if rows[r].nested.is_some() {
                let closed = f.use_var(vars.closed);
                let c = f.ins().ushr_imm(closed, r as i64);
                let c = f.ins().band_imm(c, 1);
                let not = f.ins().bxor_imm(c, 1);
                ends = f.ins().band(ends, not);
            }
            let yes = f.create_block();
            branch(&mut f, ends, yes, ends_done);
            f.switch_to_block(yes);
            let closed = f.use_var(vars.closed);
            let c = f.ins().bor_imm(closed, 1i64 << r);
            f.def_var(vars.closed, c);
            if rows[r].leaf {
                let at = f.use_var(vars.at);
                let e = f.ins().iadd_imm(at, 1);
                f.def_var(vars.leaf_end, e);
                let top = f.use_var(vars.top);
                let kind = f.ins().ushr_imm(top, 48);
                f.def_var(vars.ended, kind);
            }
            f.ins().jump(closers[r], &[]);
        }

        // Closing row `r`'s node, the stack's top: its end, size and checks.
        for (r, &closer) in closers.iter().enumerate() {
            let row = &rows[r];
            f.switch_to_block(closer);
            let sp = f.use_var(vars.sp);
            let sp1 = f.ins().iadd_imm(sp, -1);
            f.def_var(vars.sp, sp1);
            if row.nested.is_some() {
                let d = f.use_var(vars.depth);
                let d1 = f.ins().iadd_imm(d, -1);
                f.def_var(vars.depth, d1);
            }
            let top = f.use_var(vars.top);
            let ord = f.ins().band_imm(top, 0xffff_ffff);
            let off = f.ins().ishl_imm(ord, 4);
            let node = f.ins().iadd(nodes, off);
            let at = f.use_var(vars.at_close);
            let end = f.ins().iadd_imm(at, 1);
            f.ins().istore32(flags, end, node, 8);
            let n = f.use_var(vars.n);
            let size = f.ins().isub(n, ord);
            let size = if row.extra { f.ins().bor_imm(size, 1 << 31) } else { size };
            f.ins().istore32(flags, size, node, 12);
            if let Some((kind, v)) = row.check {
                let k = f.ins().ushr_imm(top, 48);
                let is = f.ins().icmp_imm(IntCC::Equal, k, i64::from(kind));
                let check = f.create_block();
                let checked = f.create_block();
                f.ins().brif(is, check, &[], checked, &[]);
                f.switch_to_block(check);
                let start = f.ins().uload32(flags, node, 4);
                let text = f.ins().iadd(src, start);
                let length = f.ins().isub(end, start);
                let callee = f.ins().iconst(pointer, validator(v) as i64);
                let call = f.ins().call_indirect(vsig, callee, &[text, length]);
                let valid = f.inst_results(call)[0];
                ensure(&mut f, valid);
                f.ins().jump(checked, &[]);
                f.switch_to_block(checked);
            }
            // A start waiting for this node, its first child, now waits for the next token.
            for (i, &(_, pend, child, opened)) in vars.pend.iter().enumerate() {
                let (wait, wait_opened) = vars.wait[i];
                let c = f.use_var(child);
                let is = f.ins().icmp(IntCC::Equal, c, ord);
                let is = f.ins().uextend(types::I64, is);
                let pv = f.use_var(pend);
                let hit = f.ins().band(is, pv);
                let yes = f.create_block();
                let no = f.create_block();
                f.ins().brif(hit, yes, &[], no, &[]);
                f.switch_to_block(yes);
                f.def_var(wait, one);
                let o = f.use_var(opened);
                f.def_var(wait_opened, o);
                f.def_var(pend, zero);
                f.ins().jump(no, &[]);
                f.switch_to_block(no);
            }
            let field = f.ins().uload16(types::I64, flags, node, 2);
            f.def_var(vars.last_field, field);
            let mode = f.use_var(vars.mode);
            f.ins().brif(mode, eof_head, &[], with_head, &[]);
        }

        // Each node that ends with the child just closed.
        f.switch_to_block(with_head);
        let sp = f.use_var(vars.sp);
        let has = f.create_block();
        f.ins().brif(sp, has, &[], end_head, &[]);
        f.switch_to_block(has);
        let top = top_of(&mut f, &vars);
        let row = f.ins().ushr_imm(top, 32);
        let row = f.ins().band_imm(row, 0xffff);
        let withs: Vec<Block> = rows.iter().map(|_| f.create_block()).collect();
        table(&mut f, row, end_head, &withs);
        for (r, &b) in withs.iter().enumerate() {
            f.switch_to_block(b);
            match rows[r].end {
                RowEnd::With(field) if field != 0 => {
                    let last = f.use_var(vars.last_field);
                    let is = f.ins().icmp_imm(IntCC::Equal, last, i64::from(field));
                    let yes = f.create_block();
                    f.ins().brif(is, yes, &[], end_head, &[]);
                    f.switch_to_block(yes);
                    let top = top_of(&mut f, &vars);
                    f.def_var(vars.top, top);
                    f.ins().jump(closers[r], &[]);
                }
                _ => {
                    f.ins().jump(end_head, &[]);
                }
            }
        }

        // A nested row's end that closed nothing: a stray or misnested close.
        f.switch_to_block(ends_done);
        for (r, row) in rows.iter().enumerate() {
            if row.nested.is_none() {
                continue;
            }
            if let Some(ends) = ends_here(&mut f, &vars, r) {
                let closed = f.use_var(vars.closed);
                let c = f.ins().ushr_imm(closed, r as i64);
                let c = f.ins().band_imm(c, 1);
                let not = f.ins().bxor_imm(c, 1);
                let stray = f.ins().band(ends, not);
                let ok = f.ins().icmp_imm(IntCC::Equal, stray, 0);
                let ok = f.ins().uextend(types::I64, ok);
                ensure(&mut f, ok);
            }
        }
        f.ins().jump(ev_head, &[]);

        // The file's end: a token, every byte in one or skipped, and the nodes that end with the
        // last token.
        f.switch_to_block(eof);
        let first = f.use_var(vars.first);
        ensure(&mut f, first);
        let leaf_end = f.use_var(vars.leaf_end);
        skipped(&mut f, leaf_end, len);
        let leaf_end = f.use_var(vars.leaf_end);
        let last = f.ins().iadd_imm(leaf_end, -1);
        f.def_var(vars.at_close, last);
        f.def_var(vars.mode, one);
        f.ins().jump(eof_head, &[]);
        f.switch_to_block(eof_head);
        let sp = f.use_var(vars.sp);
        let has = f.create_block();
        f.ins().brif(sp, has, &[], finish, &[]);
        f.switch_to_block(has);
        let top = top_of(&mut f, &vars);
        f.def_var(vars.top, top);
        let row = f.ins().ushr_imm(top, 32);
        let row = f.ins().band_imm(row, 0xffff);
        let lasts: Vec<Block> = rows.iter().enumerate().map(|(r, row)| if row.end == RowEnd::LastToken { closers[r] } else { fail }).collect();
        table(&mut f, row, fail, &lasts);

        // Done: no `before` check still waiting on a token that never came.
        f.switch_to_block(finish);
        for (i, &(_, pend, _, _)) in vars.pend.iter().enumerate() {
            let (wait, opened) = vars.wait[i];
            let w = f.use_var(wait);
            let o = f.use_var(opened);
            let bad = f.ins().band(w, o);
            let p = f.use_var(pend);
            let bad = f.ins().bor(bad, p);
            let ok = f.ins().icmp_imm(IntCC::Equal, bad, 0);
            let ok = f.ins().uextend(types::I64, ok);
            ensure(&mut f, ok);
        }
        let n = f.use_var(vars.n);
        f.ins().store(flags, n, counts, 0);
        let ns = f.use_var(vars.ns);
        f.ins().store(flags, ns, counts, 8);
        f.ins().return_(&[zero]);

        f.switch_to_block(fail);
        let failed = f.ins().iconst(types::I64, 1);
        f.ins().return_(&[failed]);
        f.seal_all_blocks();
        f.finalize();
    }
    let clif = context.func.display().to_string();
    super::native_cpu_codegen::define_function(&mut module, id, &mut context).map_err(|e| format!("define: {e:?}\n{clif}"))?;
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|e| format!("finalize: {e}"))?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_tier2` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };

    // The columns' bounds.
    let mut start_outputs: Vec<(usize, usize)> = Vec::new();
    for k in start_out.iter().flatten() {
        match start_outputs.iter_mut().find(|(o, _)| o == k) {
            Some((_, n)) => *n += 1,
            None => start_outputs.push((*k, 1)),
        }
    }
    let mut leaf_outputs: Vec<usize> = leaves.iter().filter_map(|&r| start_out[r]).collect();
    leaf_outputs.sort_unstable();
    leaf_outputs.dedup();
    let per_token = rows.iter().filter(|r| matches!(r.start, RowStart::Kind(_))).count();
    let first_rows = rows.iter().filter(|r| r.start == RowStart::FirstToken).count();
    let max_classes = (0..rows.len()).map(|r| p.codes.iter().filter(|c| usize::from(c.node) == r && !c.field).count()).max().unwrap_or(0).max(1);
    Ok(CompiledTier2 { _module: module, entry, outputs, start_outputs, leaf_outputs, per_token, first_rows, max_classes, clif })
}

/// Open a node of row `r` at the event's byte: the depth check, its code's field and
/// supertypes, the one-root check, the node, the stack. `kind`: the token's kind, if not the
/// row's.
#[allow(clippy::too_many_arguments)]
fn open_node(
    f: &mut FunctionBuilder<'_>,
    vars: &Vars,
    p: &BlockProgram,
    r: usize,
    kind: Option<Variable>,
    nodes: Value,
    supers: Value,
    stack_base: Value,
    fail: Block,
    fact_out: &dyn Fn(&Fact) -> Result<Option<usize>, String>,
    bit_of: &dyn Fn(&mut FunctionBuilder<'_>, &Vars, usize) -> Value,
) -> Result<(), String> {
    let row = &p.nodes[r];
    let flags = MemFlags::trusted();
    let ensure = |f: &mut FunctionBuilder<'_>, c: Value| {
        let ok = f.create_block();
        f.ins().brif(c, ok, &[], fail, &[]);
        f.switch_to_block(ok);
    };
    if let Some(max) = row.nested {
        let d = f.use_var(vars.depth);
        let d1 = f.ins().iadd_imm(d, 1);
        f.def_var(vars.depth, d1);
        let ok = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, d1, i64::from(max));
        let ok = f.ins().uextend(types::I64, ok);
        ensure(f, ok);
    }
    // One root: a node opens on an empty stack only as the first.
    let sp = f.use_var(vars.sp);
    let n = f.use_var(vars.n);
    let empty = f.ins().icmp_imm(IntCC::Equal, sp, 0);
    let later = f.ins().icmp_imm(IntCC::NotEqual, n, 0);
    let second = f.ins().band(empty, later);
    let ok = f.ins().bxor_imm(second, 1);
    let ok = f.ins().uextend(types::I64, ok);
    ensure(f, ok);
    // The code's facts, at the node's start.
    let holds = |f: &mut FunctionBuilder<'_>, fact: &Fact| -> Result<Value, String> {
        Ok(match fact {
            Fact::At(_) => bit_of(f, vars, fact_out(fact)?.expect("an output")),
            Fact::StartOf(s) | Fact::NotStartOf(s) => {
                let here = f.use_var(vars.here);
                let h = f.ins().ushr_imm(here, i64::from(*s));
                let h = f.ins().band_imm(h, 1);
                if matches!(fact, Fact::NotStartOf(_)) { f.ins().bxor_imm(h, 1) } else { h }
            }
            Fact::After(ks) => {
                let prev = f.use_var(vars.prev);
                let mut acc = f.ins().iconst(types::I64, 0);
                for &k in ks {
                    let e = f.ins().icmp_imm(IntCC::Equal, prev, i64::from(k));
                    let e = f.ins().uextend(types::I64, e);
                    acc = f.ins().bor(acc, e);
                }
                acc
            }
            Fact::Always => f.ins().iconst(types::I64, 1),
        })
    };
    // The first held field code: chosen last to first.
    let mut field = f.ins().iconst(types::I64, 0);
    for c in p.codes.iter().filter(|c| usize::from(c.node) == r && c.field).collect::<Vec<_>>().into_iter().rev() {
        let h = holds(f, &c.when)?;
        let id = f.ins().iconst(types::I64, i64::from(c.id));
        field = f.ins().select(h, id, field);
    }
    let n = f.use_var(vars.n);
    for c in p.codes.iter().filter(|c| usize::from(c.node) == r && !c.field) {
        let h = holds(f, &c.when)?;
        let ns = f.use_var(vars.ns);
        let off = f.ins().ishl_imm(ns, 3);
        let a = f.ins().iadd(supers, off);
        let entry = f.ins().bor_imm(n, i64::from(c.id) << 32);
        f.ins().store(flags, entry, a, 0);
        let ns1 = f.ins().iadd(ns, h);
        f.def_var(vars.ns, ns1);
    }
    let kind = match kind {
        Some(v) => f.use_var(v),
        None => f.ins().iconst(types::I64, i64::from(row.kind)),
    };
    let off = f.ins().ishl_imm(n, 4);
    let node = f.ins().iadd(nodes, off);
    f.ins().istore16(flags, kind, node, 0);
    f.ins().istore16(flags, field, node, 2);
    let at = f.use_var(vars.at);
    f.ins().istore32(flags, at, node, 4);
    let sp = f.use_var(vars.sp);
    let off = f.ins().ishl_imm(sp, 3);
    let slot = f.ins().iadd(stack_base, off);
    let k = f.ins().ishl_imm(kind, 48);
    let entry = f.ins().bor_imm(n, (r as i64) << 32);
    let entry = f.ins().bor(entry, k);
    f.ins().store(flags, entry, slot, 0);
    let sp1 = f.ins().iadd_imm(sp, 1);
    f.def_var(vars.sp, sp1);
    let n1 = f.ins().iadd_imm(n, 1);
    f.def_var(vars.n, n1);
    Ok(())
}

/// A built-in validator over a token's text.
pub(super) fn valid(v: Validator, text: &[u8]) -> bool {
    match v {
        Validator::JsonNumber => json_number_text(text),
    }
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE]-?[0-9]+)?`: tree-sitter-json's number (its exponent takes no `+`).
fn json_number_text(t: &[u8]) -> bool {
    let digits = |t: &[u8], i: usize| t[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut i = usize::from(t.first() == Some(&b'-'));
    match t.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => i += digits(t, i),
        _ => return false,
    }
    if t.get(i) == Some(&b'.') {
        let n = digits(t, i + 1);
        if n == 0 {
            return false;
        }
        i += 1 + n;
    }
    if matches!(t.get(i), Some(b'e' | b'E')) {
        i += 1;
        if t.get(i) == Some(&b'-') {
            i += 1;
        }
        let n = digits(t, i);
        if n == 0 {
            return false;
        }
        i += n;
    }
    i == t.len()
}
