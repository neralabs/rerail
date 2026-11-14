//! Generated code read as text, for the checks of its shape: the hot loop in Cranelift IR,
//! an instruction's opcode, the hot loop in machine code and its stack traffic; and a block
//! program's carried state, as its shape checks count it.

/// The CLIF blocks of the hot loop, each with its instruction lines: the full body (`block2`) and
/// the blocks it reaches before it leaves for the head (`block4`) or loops back to itself, which
/// are its extracts' flattening.
pub fn hot_loop(clif: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for line in clif.lines() {
        let t = line.trim();
        if t.starts_with("block") && t.ends_with(':') {
            let name = t.split(['(', ':']).next().unwrap_or("").to_owned();
            blocks.push((name, Vec::new()));
        } else if let Some((_, lines)) = blocks.last_mut()
            && !t.is_empty()
        {
            lines.push(t.to_owned());
        }
    }
    let successors = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| matches!(opcode(l), "brif" | "jump"))
            .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()).filter(|w| w.starts_with("block")).map(str::to_owned).collect::<Vec<_>>())
            .collect()
    };
    let (mut seen, mut work, mut out) = (vec!["block2".to_owned()], vec!["block2".to_owned()], Vec::new());
    while let Some(name) = work.pop() {
        let (_, lines) = blocks.iter().find(|(n, _)| *n == name).expect("a block of the function");
        for next in successors(lines) {
            if next != "block4" && !seen.contains(&next) {
                seen.push(next.clone());
                work.push(next);
            }
        }
        out.push((name, lines.clone()));
    }
    out
}

/// The CLIF opcode of an instruction line (`v3 = icmp eq v1, v2` → `icmp`).
pub fn opcode(line: &str) -> &str {
    let line = line.split(" ;").next().unwrap_or(line);
    let rhs = line.split_once(" = ").map_or(line, |(_, r)| r);
    rhs.split([' ', '.']).next().unwrap_or("")
}

/// The hot loop of a compiled block function's machine code: the full blocks' body, from the
/// block its back edge takes to that edge, in layout order (the flattening's loops inside it
/// counted once). Each loop is a back edge, block `j`'s last branch to block `i <= j`. The body's
/// is the one that leaves (through edge blocks, each ending in one `b`) to the head of a loop around it (the
/// blocks' head); the files' loop leaves to the return, a flattening's into the body. Labels are a function's
/// own: each function starts at `block0`.
pub fn machine_loop(disassembly: &str) -> Vec<String> {
    let mut functions: Vec<Vec<(String, Vec<String>)>> = Vec::new();
    for line in disassembly.lines() {
        if let Some(label) = line.strip_suffix(':').filter(|l| l.starts_with("block")) {
            let label = label.trim_start_matches("block").to_owned();
            if label == "0" || functions.is_empty() {
                functions.push(Vec::new());
            }
            functions.last_mut().expect("a function").push((label, Vec::new()));
        } else if let Some((_, body)) = functions.last_mut().and_then(|f| f.last_mut()) {
            body.push(line.trim().to_owned());
        }
    }
    let mut best: Vec<String> = Vec::new();
    for blocks in &functions {
        let index = |label: &str| blocks.iter().position(|(l, _)| l == label);
        // Each block's branch targets, as block indexes.
        let exits: Vec<Vec<usize>> = blocks
            .iter()
            .map(|(_, lines)| {
                let tail = lines.iter().rev().take(2).cloned().collect::<Vec<_>>().join(" ");
                tail.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.strip_prefix("label")).filter_map(index).collect()
            })
            .collect();
        let loops: Vec<(usize, usize)> = (0..blocks.len()).flat_map(|j| exits[j].iter().filter(move |&&i| i <= j).map(move |&i| (i, j))).collect();
        // Through the edge blocks (a lone `b`) to where a branch goes.
        let resolve = |mut t: usize| {
            for _ in 0..blocks.len() {
                match (blocks[t].1.last(), exits[t].as_slice()) {
                    (Some(last), [next]) if last.starts_with("b label") => t = *next,
                    _ => break,
                }
            }
            t
        };
        for &(i, j) in &loops {
            let leaves: Vec<usize> = (i..=j).flat_map(|k| exits[k].iter().copied()).filter(|&t| t < i || t > j).map(resolve).collect();
            let leaves_to_head = leaves.iter().any(|&h| loops.iter().any(|&(i2, j2)| i2 == h && i2 <= i && j2 >= j && (i2, j2) != (i, j)));
            if leaves_to_head {
                let out: Vec<String> = blocks[i..=j].iter().flat_map(|(_, l)| l.iter().cloned()).collect();
                if out.len() > best.len() {
                    best = out;
                }
            }
        }
    }
    assert!(!best.is_empty(), "no hot loop");
    best
}

/// A loop's stack loads and stores (spills and reloads), and its vector `mvn`s.
pub fn loop_costs(lines: &[String]) -> (usize, usize) {
    let stack = lines.iter().filter(|l| (l.starts_with("ldr") || l.starts_with("str") || l.starts_with("ldp") || l.starts_with("stp")) && l.contains("[sp")).count();
    let mvn = lines.iter().filter(|l| l.starts_with("mvn v")).count();
    (stack, mvn)
}

/// How a machine-code instruction leaves for code elsewhere, as Cranelift prints it for the host:
/// aarch64 or x86-64.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transfer {
    /// A direct call (`bl`, `call` of a name).
    NearCall,
    /// A call through a register (`blr`, `call *`).
    FarCall,
    /// An indirect jump of its own (`br`, `jmp *`).
    Jump,
    /// A jump table's sequence, its indirect jump inside (`jt_entries`, `br_table`).
    Table,
}

/// The transfers of a line of machine code, one per instruction (a line can hold a sequence,
/// `;`-separated).
pub fn transfers(line: &str) -> Vec<Transfer> {
    if cfg!(target_arch = "aarch64") {
        if line.contains("jt_entries") {
            return vec![Transfer::Table];
        }
        line.split(';')
            .filter_map(|i| match i.split_whitespace().next() {
                Some("bl") => Some(Transfer::NearCall),
                Some("blr") => Some(Transfer::FarCall),
                Some("br") => Some(Transfer::Jump),
                _ => None,
            })
            .collect()
    } else {
        line.split(';')
            .filter_map(|i| match i.split_whitespace().collect::<Vec<_>>()[..] {
                ["br_table", ..] => Some(Transfer::Table),
                ["call", to, ..] => Some(if to.starts_with('*') { Transfer::FarCall } else { Transfer::NearCall }),
                ["jmp", to, ..] if to.starts_with('*') => Some(Transfer::Jump),
                _ => None,
            })
            .collect()
    }
}

/// How many of `disassembly`'s instructions are transfers of `kind`.
pub fn count_transfers(disassembly: &str, kind: Transfer) -> usize {
    disassembly.lines().flat_map(transfers).filter(|&t| t == kind).count()
}

/// The Cranelift IR of the loop body: everything between the body block's label and the next
/// block's. `block0` is the entry, `block1` a file's row, `block2` the body over its blocks.
pub fn body_of(clif: &str) -> Vec<String> {
    hot_loop(clif).into_iter().flat_map(|(_, lines)| lines).collect()
}

/// The values a program's `prev`s read, one carried word each, and its other carried operations.
pub fn carried_words(p: &crate::block::BlockProgram) -> usize {
    use crate::block::BlockOp::*;
    let bases: std::collections::BTreeSet<u16> = p.ops.iter().filter_map(|op| if let Prev(a, _) = *op { Some(a) } else { None }).collect();
    bases.len() + p.ops.iter().filter(|op| matches!(op, Toggle(_) | Escaped(_) | Fill(..))).count()
}

/// A program's carried state in bits (decision 0070): one bit per `toggle`, `escaped` and
/// `fill`, and for each value its `prev`s read, the top `k` bits of its previous word (`k` the
/// farthest look-back). A `toggle`'s and a `fill`'s carry is its top bit, so a look-back on one
/// costs `k - 1` more. Every executor carries these bits and no more: Cranelift packs them into
/// words, never a word per bit.
pub fn state_bits(p: &crate::block::BlockProgram) -> usize {
    use crate::block::BlockOp::*;
    let mut back: std::collections::BTreeMap<u16, u8> = std::collections::BTreeMap::new();
    for op in &p.ops {
        if let Prev(a, k) = *op {
            let m = back.entry(a).or_insert(0);
            *m = (*m).max(k);
        }
    }
    let ops = p.ops.iter().filter(|op| matches!(op, Toggle(_) | Escaped(_) | Fill(..))).count();
    let looks: usize = back
        .iter()
        .map(|(&a, &k)| match p.ops[usize::from(a)] {
            Toggle(_) | Fill(..) => usize::from(k) - 1,
            _ => usize::from(k),
        })
        .sum();
    ops + looks
}

/// The function's loop-carried values, read from the Cranelift IR: the hot body's parameters
/// after its block index and its byte and output pointers.
pub fn carried_params(clif: &str) -> usize {
    let line = clif.lines().find(|l| l.trim_start().starts_with("block2(")).expect("the body block");
    line.matches(": i64").count() - 3
}

/// The sinks' loop-carried values (`native_cpu_block`): a running count per counted output and
/// no more (a delayed output holds no word).
pub fn sink_params(native: &crate::block_native::BlockNative) -> usize {
    // A delayed output's word is not held (`native_cpu_block::delayed_store`).
    crate::native_cpu_block::counted(native).len()
}
