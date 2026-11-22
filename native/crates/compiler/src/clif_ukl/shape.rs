//! The structuring of a function's graph that `clif_ukl` prints as C (`Shape`), and the reads of a function's graph
//! it needs (`successors`, `returned_call`). It was the Rust translator's (`rerail_clif::rust`),
//! removed (decision 0569).

use cranelift_codegen::ir;
use std::fmt::Write;

/// A function's graph as structured control ("Beyond Relooper", Ramsey 2022, as WebAssembly's backends do), printed
/// as C, which has no labels: a loop header a `for (;;)` over a `do { … } while (false)`, a node two forward edges
/// reach (a merge) the code after a `do { … } while (false)` its branches break out of, any other node inline at its
/// one branch. A jump sets the target's number (`j`: a loop's `2x + 1`, a merge's `2y + 2`) and breaks; each
/// construct on the way out stops it there or breaks on.
///
/// The graph's nodes are its blocks and copies of them: where an edge goes back to a node that
/// does not dominate its source (an irreducible loop, a loop entered at two places), the edge
/// gets its own copy of its target (node splitting), until none does. A copy runs the block's
/// code: each value is a local, which its last definition set, as SSA's dominance requires.
pub(super) struct Shape {
    /// Each node's block (its place in the graph given) and its successors (nodes), in the
    /// terminator's order.
    block: Vec<Option<usize>>,
    succ: Vec<Vec<usize>>,
    /// The edges into a dispatch node (`None` its block): their target's place in its successors.
    label: std::collections::HashMap<(usize, usize), usize>,
    /// Each node's place in reverse postorder (`usize::MAX` unreachable).
    rpo: Vec<usize>,
    /// The merge nodes each node immediately dominates, the latest in the order first.
    merges: Vec<Vec<usize>>,
    header: Vec<bool>,
    merge: Vec<bool>,
}

/// Block `b`'s successors, in its terminator's order.
pub(super) fn successors(f: &ir::Function, b: ir::Block) -> Vec<ir::Block> {
    let Some(last) = f.layout.last_inst(b) else { return Vec::new() };
    f.dfg.insts[last].branch_destination(&f.dfg.jump_tables, &f.dfg.exception_tables).iter().map(|c| c.block(&f.dfg.value_lists)).collect()
}

/// The graph's order, dominators and loops: reverse postorder, each node's immediate dominator
/// (by place), its predecessors; or the first edge back to a node that does not dominate it.
/// The dominator tree numbered in preorder (`pre`, by place) with each subtree's end (`end`): `a`
/// dominates `b` where `b`'s number is in `a`'s subtree's, one test (`dominates`) where a walk up the
/// tree is as long as it is deep, for each edge of each round of `Shape::of`'s splitting.
struct Order {
    order: Vec<usize>,
    rpo: Vec<usize>,
    idom: Vec<usize>,
    preds: Vec<Vec<usize>>,
    pre: Vec<usize>,
    end: Vec<usize>,
}

impl Order {
    fn of(succ: &[Vec<usize>]) -> Order {
        let n = succ.len();
        let mut post = Vec::new();
        let mut seen = vec![false; n];
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        while let Some(top) = stack.last_mut() {
            let b = top.0;
            if let Some(&t) = succ[b].get(top.1) {
                top.1 += 1;
                if !seen[t] {
                    seen[t] = true;
                    stack.push((t, 0));
                }
            } else {
                post.push(b);
                stack.pop();
            }
        }
        let order: Vec<usize> = post.into_iter().rev().collect();
        let mut rpo = vec![usize::MAX; n];
        for (i, &b) in order.iter().enumerate() {
            rpo[b] = i;
        }
        let mut preds = vec![Vec::new(); n];
        for &b in &order {
            for &t in &succ[b] {
                preds[t].push(b);
            }
        }
        let idom = Self::idoms(succ, &order, &rpo, &preds);
        // The dominator tree in preorder, iteratively: each place's number, and its subtree's end.
        let m = order.len();
        let mut children = vec![Vec::new(); m];
        for i in 1..m {
            children[idom[i]].push(i);
        }
        let (mut pre, mut end) = (vec![0; m], vec![0; m]);
        let mut next = 0;
        let mut stack = vec![(0usize, 0usize)];
        pre[0] = 0;
        next += 1;
        while let Some(top) = stack.last_mut() {
            let (i, k) = *top;
            if let Some(&c) = children[i].get(k) {
                top.1 += 1;
                pre[c] = next;
                next += 1;
                stack.push((c, 0));
            } else {
                end[i] = next;
                stack.pop();
            }
        }
        Order { order, rpo, idom, preds, pre, end }
    }

    /// Each place's immediate dominator, by place (the entry's its own), by Semi-NCA (Georgiadis;
    /// LLVM's): semidominators by Lengauer and Tarjan's evaluation with path compression on a
    /// depth-first spanning tree, then each node's immediate dominator the nearest of its tree
    /// parent's dominators at or above its semidominator. One pass, where Cooper, Harvey and
    /// Kennedy's iteration to a fixed point takes many on an irreducible graph.
    fn idoms(succ: &[Vec<usize>], order: &[usize], rpo: &[usize], preds: &[Vec<usize>]) -> Vec<usize> {
        const NONE: usize = usize::MAX;
        let n = succ.len();
        // The spanning tree: each node's preorder number, the node at each, each one's parent.
        let mut num = vec![NONE; n];
        let (mut vertex, mut parent) = (vec![0usize], vec![0usize]);
        num[0] = 0;
        let mut stack = vec![(0usize, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (b, k) = *top;
            if let Some(&t) = succ[b].get(k) {
                top.1 += 1;
                if num[t] == NONE {
                    num[t] = vertex.len();
                    parent.push(num[b]);
                    vertex.push(t);
                    stack.push((t, 0));
                }
            } else {
                stack.pop();
            }
        }
        let m = vertex.len();
        let mut semi: Vec<usize> = (0..m).collect();
        let mut label: Vec<usize> = (0..m).collect();
        let mut ancestor = vec![NONE; m];
        let mut path = Vec::new();
        for w in (1..m).rev() {
            for &v in &preds[vertex[w]] {
                let v = num[v];
                // `eval(v)`: the least semidominator on `v`'s path in the linked forest, the path
                // compressed on the way.
                let u = if ancestor[v] == NONE {
                    v
                } else {
                    let mut x = v;
                    while ancestor[ancestor[x]] != NONE {
                        path.push(x);
                        x = ancestor[x];
                    }
                    while let Some(x) = path.pop() {
                        let a = ancestor[x];
                        if semi[label[a]] < semi[label[x]] {
                            label[x] = label[a];
                        }
                        ancestor[x] = ancestor[a];
                    }
                    label[v]
                };
                semi[w] = semi[w].min(semi[u]);
            }
            ancestor[w] = parent[w];
        }
        let mut idom = parent;
        for w in 1..m {
            while idom[w] > semi[w] {
                idom[w] = idom[idom[w]];
            }
        }
        // By place in the order.
        let mut by_place = vec![0; order.len()];
        for (i, &b) in order.iter().enumerate().skip(1) {
            by_place[i] = rpo[vertex[idom[num[b]]]];
        }
        by_place
    }

    /// Whether place `a` dominates place `b`: `b` in `a`'s subtree of the dominator tree.
    fn dominates(&self, a: usize, b: usize) -> bool {
        self.pre[a] <= self.pre[b] && self.pre[b] < self.end[a]
    }

    /// An edge (its source node and its place in the source's successors) back to a node that
    /// does not dominate its source.
    fn irreducible(&self, succ: &[Vec<usize>]) -> Option<(usize, usize)> {
        for &x in &self.order {
            for (k, &t) in succ[x].iter().enumerate() {
                if self.rpo[t] <= self.rpo[x] && !self.dominates(self.rpo[t], self.rpo[x]) {
                    return Some((x, k));
                }
            }
        }
        None
    }
}

impl Shape {
    /// The shape of a graph (its nodes' successors, the entry node 0, and each node's size: its copies' cost): its
    /// nodes split until it is reducible, within its own size again in copies; past that, each loop with two entries
    /// given one, a dispatch.
    pub(super) fn of(first: Vec<Vec<usize>>, sizes: &[usize]) -> Option<Shape> {
        let size = |b: usize| sizes[b];
        let budget: usize = sizes.iter().sum();
        // Split within the budget; past it, one dispatch for the outermost loops entered at two places, and the
        // loops within them split (as the lean parser's one `match label`, its `ScopeEnd` a copy of each container's
        // continue); past that again, a dispatch for every such loop.
        // An edge into a loop entered at two places, where a dispatch holds it and the entry branches, goes to the
        // dispatch, as the lean parser's `break Label::ArrayValue` from its `ScopeEnd`; any other is given a copy of
        // its target (an entry that only goes on to one place, as a `TOKEN` before a switch: its copy is that much).
        let split = |mut block: Vec<Option<usize>>, mut succ: Vec<Vec<usize>>, mut label: std::collections::HashMap<(usize, usize), usize>| {
            let mut copied = 0;
            loop {
                let order = Order::of(&succ);
                let Some((x, k)) = order.irreducible(&succ) else { return Some((block, succ, label, order)) };
                let t = succ[x][k];
                if let Some(d) = Self::dispatch_over(&block, &order, x).filter(|_| succ[t].len() > 1 && Self::two_entries(&block, &succ).contains(&t)) {
                    let j = succ[d].iter().position(|&e| e == t).unwrap_or_else(|| {
                        succ[d].push(t);
                        succ[d].len() - 1
                    });
                    label.insert((x, k), j);
                    succ[x][k] = d;
                    continue;
                }
                copied += block[t].map_or(budget + 1, size);
                if copied > budget {
                    return None;
                }
                let c = block.len();
                block.push(block[t]);
                succ.push(succ[t].clone());
                let carried: Vec<(usize, usize)> = label.iter().filter(|&(&(u, _), _)| u == t).map(|(&(_, e), &l)| (e, l)).collect();
                label.extend(carried.into_iter().map(|(e, l)| ((c, e), l)));
                succ[x][k] = c;
            }
        };
        let plain: Vec<Option<usize>> = (0..first.len()).map(Some).collect();
        let (block, succ, label, order) = match split(plain.clone(), first.clone(), std::collections::HashMap::new()) {
            Some(done) => done,
            None => {
                let (b, s, l) = Self::dispatched(plain.clone(), first.clone(), true)?;
                match split(b, s, l) {
                    Some(done) => done,
                    None => {
                        let (block, succ, label) = Self::dispatched(plain, first.clone(), false)?;
                        let order = Order::of(&succ);
                        if order.irreducible(&succ).is_some() {
                            return None;
                        }
                        (block, succ, label, order)
                    }
                }
            }
        };
        let n = block.len();
        let (mut header, mut merge) = (vec![false; n], vec![false; n]);
        for &b in &order.order {
            let i = order.rpo[b];
            let forward = order.preds[b].iter().filter(|&&p| order.rpo[p] < i).count();
            header[b] = order.preds[b].iter().any(|&p| order.rpo[p] >= i);
            merge[b] = forward >= 2;
        }
        let mut merges = vec![Vec::new(); n];
        for &b in order.order.iter().skip(1) {
            if merge[b] {
                merges[order.order[order.idom[order.rpo[b]]]].push(b);
            }
        }
        for m in &mut merges {
            m.sort_by_key(|&b| std::cmp::Reverse(order.rpo[b]));
        }
        Some(Shape { block, succ, label, rpo: order.rpo, merges, header, merge })
    }

    /// The dispatch nearest above node `x` in the dominator tree.
    fn dispatch_over(block: &[Option<usize>], order: &Order, x: usize) -> Option<usize> {
        let mut i = order.rpo[x];
        loop {
            let b = order.order[i];
            if block[b].is_none() {
                return Some(b);
            }
            if i == 0 {
                return None;
            }
            i = order.idom[i];
        }
    }

    /// The entries of each loop entered at two places or more, the dispatches left out (a loop's
    /// nodes a strongly connected part; its entries the nodes an edge from outside it reaches).
    fn two_entries(block: &[Option<usize>], succ: &[Vec<usize>]) -> std::collections::HashSet<usize> {
        let out: Vec<bool> = block.iter().map(Option::is_none).collect();
        let mut all = std::collections::HashSet::new();
        for part in strong_parts(succ, &out) {
            let inside: std::collections::HashSet<usize> = part.iter().copied().collect();
            let entries: std::collections::HashSet<usize> = succ.iter().enumerate().filter(|(u, _)| !inside.contains(u)).flat_map(|(_, next)| next.iter().copied().filter(|t| inside.contains(t))).collect();
            if entries.len() >= 2 {
                all.extend(entries);
            }
        }
        all
    }

    /// Each loop (a strongly connected part of the graph, its header out, then the parts within,
    /// as a loop forest) entered at two nodes or more given a dispatch node, its one entry: the
    /// edges into those nodes go to the dispatch, each with its target's place (`label`), the
    /// dispatch on to it. `None` where the function's first block is one of two entries. `outer`: only the
    /// outermost such loops, those no loop entered at two places holds.
    #[allow(clippy::type_complexity)]
    fn dispatched(mut block: Vec<Option<usize>>, mut succ: Vec<Vec<usize>>, outer: bool) -> Option<(Vec<Option<usize>>, Vec<Vec<usize>>, std::collections::HashMap<(usize, usize), usize>)> {
        let mut label = std::collections::HashMap::new();
        let mut out = vec![false; block.len()];
        loop {
            let parts = strong_parts(&succ, &out);
            if parts.is_empty() || (outer && !label.is_empty()) {
                return Some((block, succ, label));
            }
            for part in parts {
                let inside: std::collections::HashSet<usize> = part.iter().copied().collect();
                let mut entries: Vec<usize> = Vec::new();
                for (u, next) in succ.iter().enumerate() {
                    for &t in next {
                        if inside.contains(&t) && !inside.contains(&u) && !entries.contains(&t) {
                            entries.push(t);
                        }
                    }
                }
                if inside.contains(&0) && !entries.contains(&0) {
                    entries.push(0);
                }
                if entries.len() == 1 {
                    out[entries[0]] = true;
                    continue;
                }
                if entries.contains(&0) {
                    return None;
                }
                let d = block.len();
                for (u, next) in succ.iter_mut().enumerate() {
                    for (k, t) in next.iter_mut().enumerate() {
                        if let Some(j) = entries.iter().position(|&e| e == *t) {
                            label.insert((u, k), j);
                            *t = d;
                        }
                    }
                }
                block.push(None);
                succ.push(entries);
                out.push(true);
            }
        }
    }

    /// The code of node `x` and the nodes it dominates: each node's own code by `emit`, given the node (its place in
    /// the graph given) and its branches' code (`jump(k)`, its `k`th successor's).
    pub(super) fn tree(&self, emit: Emit<'_>, x: usize) -> Result<String, String> {
        let within = self.within(emit, x, &self.merges[x])?;
        Ok(if self.header[x] { format!("for (;;) {{\ndo {{\n{within}}} while (false);\nif (j != 0u && j != {}u) break;\nj = 0u;\n}}\n", 2 * x + 1) } else { within })
    }

    /// Node `x`'s `k`th edge: its dispatch's entry (`d`), then a jump.
    fn transfer(&self, emit: Emit<'_>, x: usize, k: usize) -> Result<String, String> {
        let to = self.succ[x][k];
        let jump = if self.rpo[to] <= self.rpo[x] {
            format!("{{ j = {}u; break; }}", 2 * to + 1)
        } else if self.merge[to] {
            format!("{{ j = {}u; break; }}", 2 * to + 2)
        } else {
            self.tree(emit, to)?
        };
        Ok(match self.label.get(&(x, k)) {
            Some(&j) => format!("d = {j}; {jump}"),
            None => jump,
        })
    }

    fn within(&self, emit: Emit<'_>, x: usize, ys: &[usize]) -> Result<String, String> {
        if let Some((&y, rest)) = ys.split_first() {
            let inner = self.within(emit, x, rest)?;
            let after = self.tree(emit, y)?;
            return Ok(format!("do {{\n{inner}}} while (false);\nif (j != 0u && j != {}u) break;\nj = 0u;\n{after}", 2 * y + 2));
        }
        let Some(b) = self.block[x] else {
            // A dispatch: a jump in a case breaks the `switch`, then the construct around it.
            let mut s = String::from("switch (d) {\n");
            for k in 0..self.succ[x].len() {
                writeln!(s, "case {k}u: {{ {} }} break;", self.transfer(emit, x, k)?).unwrap();
            }
            s.push_str("default: break;\n}\n");
            return Ok(s);
        };
        emit(b, &|k| self.transfer(emit, x, k))
    }
}

/// A node's own code (`Shape::tree`): the node, and its branches' code.
pub(super) type Emit<'a> = &'a dyn Fn(usize, &dyn Fn(usize) -> Result<String, String>) -> Result<String, String>;

/// The graph's strongly connected parts with a cycle, the nodes `out` left out (iterative
/// Tarjan).
fn strong_parts(succ: &[Vec<usize>], out: &[bool]) -> Vec<Vec<usize>> {
    let n = succ.len();
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut parts, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if out[root] || index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(top) = work.last_mut() {
            let v = top.0;
            if let Some(&w) = succ[v].get(top.1) {
                top.1 += 1;
                if out[w] {
                    continue;
                }
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(u, _)) = work.last() {
                    low[u] = low[u].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut part = Vec::new();
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        part.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if part.len() > 1 || succ[v].contains(&v) {
                        parts.push(part);
                    }
                }
            }
        }
    }
    parts
}

/// Whether call `inst` is followed by a return of its results, in order, and nothing else:
/// `return f(…)`.
pub(super) fn returned_call(f: &ir::Function, inst: ir::Inst) -> bool {
    let dfg = &f.dfg;
    if dfg.insts[inst].opcode() != ir::Opcode::Call {
        return false;
    }
    f.layout.next_inst(inst).is_some_and(|r| dfg.insts[r].opcode() == ir::Opcode::Return && dfg.inst_args(r) == dfg.inst_results(inst) && !dfg.inst_results(inst).is_empty())
}

#[cfg(test)]
mod tests {
    use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, types::I64};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    /// `entry` branches into a loop of `a` and `b` at either: a loop with two entries (no
    /// block dominates the other), so the graph is irreducible.
    fn two_entries() -> cranelift_codegen::ir::Function {
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 0), sig);
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let (entry, x, y, done) = (b.create_block(), b.create_block(), b.create_block(), b.create_block());
        b.append_block_params_for_function_params(entry);
        for blk in [x, y, done] {
            b.append_block_param(blk, I64);
        }
        b.switch_to_block(entry);
        let n = b.block_params(entry)[0];
        let odd = b.ins().band_imm(n, 1);
        b.ins().brif(odd, x, &[n.into()], y, &[n.into()]);
        b.switch_to_block(x);
        let v = b.block_params(x)[0];
        let v = b.ins().iadd_imm(v, -1);
        b.ins().jump(y, &[v.into()]);
        b.switch_to_block(y);
        let v = b.block_params(y)[0];
        let v = b.ins().iadd_imm(v, -1);
        let more = b.ins().icmp_imm(cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan, v, 0);
        b.ins().brif(more, x, &[v.into()], done, &[v.into()]);
        b.switch_to_block(done);
        let v = b.block_params(done)[0];
        b.ins().return_(&[v]);
        b.seal_all_blocks();
        b.finalize();
        f
    }

    /// `Order::of`'s dominators are the definition's (a node's are itself and those of all its
    /// predecessors), on random graphs, irreducible ones among them: each node's immediate
    /// dominator, and `dominates` for every pair.
    #[test]
    fn the_dominators_are_the_definitions() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n) as usize
        };
        for _ in 0..300 {
            let n = 2 + next(30);
            let succ: Vec<Vec<usize>> = (0..n).map(|_| (0..next(4)).map(|_| next(n as u64)).collect()).collect();
            let order = super::Order::of(&succ);
            let m = order.order.len();
            // By the definition, over the places reached: each place's dominators, to a fixed point.
            let mut dom: Vec<Vec<bool>> = (0..m).map(|i| (0..m).map(|j| i == 0 && j == 0 || i != 0).collect()).collect();
            let mut changed = true;
            while changed {
                changed = false;
                for i in 1..m {
                    let mut d = vec![true; m];
                    for &p in &order.preds[order.order[i]] {
                        for (j, x) in d.iter_mut().enumerate() {
                            *x &= dom[order.rpo[p]][j];
                        }
                    }
                    d[i] = true;
                    if d != dom[i] {
                        dom[i] = d;
                        changed = true;
                    }
                }
            }
            for (i, of) in dom.iter().enumerate() {
                for (j, &d) in of.iter().enumerate() {
                    assert_eq!(order.dominates(j, i), d, "{succ:?}: does place {j} dominate {i}?");
                }
                if i > 0 {
                    // The immediate one: a strict dominator every other strict one dominates.
                    let strict: Vec<usize> = (0..m).filter(|&j| j != i && of[j]).collect();
                    let want = *strict.iter().find(|&&d| strict.iter().all(|&e| dom[d][e])).expect("an immediate dominator");
                    assert_eq!(order.idom[i], want, "{succ:?}: place {i}'s immediate dominator");
                }
            }
        }
    }

    /// A loop with two entries is structured (its node split), not a dispatch over the blocks: each block's code is
    /// written, inside a loop.
    #[test]
    fn a_loop_with_two_entries_is_structured() {
        let f = two_entries();
        let blocks: Vec<cranelift_codegen::ir::Block> = f.layout.blocks().collect();
        let node_of: std::collections::HashMap<_, _> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let first: Vec<Vec<usize>> = blocks.iter().map(|&b| super::successors(&f, b).iter().map(|t| node_of[t]).collect()).collect();
        let sizes: Vec<usize> = blocks.iter().map(|&b| f.layout.block_insts(b).count()).collect();
        let shape = super::Shape::of(first.clone(), &sizes).expect("structured");
        assert!(shape.block.len() > blocks.len(), "a node split: {:?}", shape.block);
        assert!(shape.block.iter().all(Option::is_some), "no dispatch: {:?}", shape.block);
        let emit = |node: usize, jump: &dyn Fn(usize) -> Result<String, String>| -> Result<String, String> {
            let mut s = format!("b{node};\n");
            for k in 0..first[node].len() {
                s.push_str(&jump(k)?);
                s.push('\n');
            }
            Ok(s)
        };
        let c = shape.tree(&emit, 0).expect("prints");
        assert!(c.contains("for (;;)"), "{c}");
        assert!(!c.contains("switch"), "{c}");
        for b in 0..blocks.len() {
            assert!(c.contains(&format!("b{b};")), "block {b}: {c}");
        }
    }

    /// A call whose results are returned as they are is the return's (`returned_call`): `return f(…)`. A call
    /// another follows is not.
    #[test]
    fn a_returned_call_is_the_return() {
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 1), sig.clone());
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let n = b.block_params(entry)[0];
        let name = b.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, 0));
        let signature = b.import_signature(sig);
        let r = b.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        b.ins().call(r, &[n]);
        let last = b.ins().call(r, &[n]);
        let got = b.inst_results(last).to_vec();
        b.ins().return_(&got);
        b.seal_all_blocks();
        b.finalize();
        let calls: Vec<cranelift_codegen::ir::Inst> = f.layout.block_insts(f.layout.entry_block().unwrap()).filter(|&i| f.dfg.insts[i].opcode() == cranelift_codegen::ir::Opcode::Call).collect();
        assert!(!super::returned_call(&f, calls[0]), "a call another follows");
        assert!(super::returned_call(&f, calls[1]), "the call the return returns");
    }
}
