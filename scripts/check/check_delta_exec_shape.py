#!/usr/bin/env python3
"""The delta executor's shape checks (decision 0686, the rebuild plan's §5).

Each milestone of the rebuild adds the mechanical checks it can make here: names the final shape has no place for,
and forms the agreed design rules out. A milestone's commit runs this beside check_no_block_copies.py, and its
checklist names the result. Run from the repository root:

    python3 scripts/check/check_delta_exec_shape.py

Exit 0 when every check holds; otherwise each failure is printed, with its file and line, and the exit is 1.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CRATE = ROOT / "native/crates/delta_exec"
TWINS = ROOT / "native/crates/delta_twins"
CHANGE = ROOT / "native/crates/blocks/src/change.rs"


def strip_rust(src: str) -> str:
    """Rust with its comments and string literals blanked, line breaks kept."""
    out, i, n = [], 0, len(src)
    while i < n:
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * src.count("\n", i, j))
            i = j
        elif src[i] == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append('""')
            i = j + 1
        else:
            out.append(src[i])
            i += 1
    return "".join(out)


def strip_rerail(src: str) -> str:
    """A .rerail file with its comments and strings blanked, line breaks kept."""
    lines = []
    for line in src.split("\n"):
        line = re.sub(r'"(\\.|[^"\\])*"', '""', line)
        lines.append(line.split("//", 1)[0])
    return "\n".join(lines)


def hits(path: pathlib.Path, text: str, pattern: str, why: str, failures: list) -> None:
    for no, line in enumerate(text.split("\n"), 1):
        if re.search(pattern, line):
            failures.append(f"{path.relative_to(ROOT)}:{no}: {why}: {line.strip()}")


def m0(failures: list) -> None:
    """M0, the clean slate: what the first build had and the final shape has no place for."""
    forbidden = [
        (r"\bBoot\b", "a driver of fixed levels (the loop is B§8's, M8)"),
        (r"\bExec\b", "the first build's executor"),
        (r"\bstruct\s+Rounds\b|\bRounds\s*(::|<|\{)", "a second table type for a fixpoint's rounds (Across::Rounds, the axis, is a phase's)"),
        (r"\bsettle\b", "a roll-up that rewrites rows (traces roll up geometrically, M1)"),
        (r"\bDb\b", "a store of its own beside the traces"),
        (r"with_prefix|\bk1_", "a level's program named by a prefix (the export is K-form, O6)"),
        (r"\bk_fn\b|\beval\b", "arithmetic in K0 (O2)"),
        (r"\bstratum\b", "strata stated per head (strata are files, O4)"),
        (r"change::body|\bPart::|\bBody::", "the k-way body operator (steps are separate operators, O1)"),
        (r"write_pair|write_diff|write_keys", "the first build's writes"),
        (r"\bWorld\b|std::fs::|File::open|read_to_string",
         "a file read of the crate's own (files come through the engine's file layer: the observer, placed reads)"),
    ]
    for path in sorted((CRATE / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        for pattern, why in forbidden:
            hits(path, code, pattern, why, failures)
    for path in sorted(CRATE.rglob("*.rerail")):
        code = strip_rerail(path.read_text())
        hits(path, code, r"[\w)]\s*[-+*/%]\s*[\w(]", "arithmetic (K0 has none, O2)", failures)
        hits(path, code, r"\bstratum\s*\(", "a stated stratum (strata are files, O4)", failures)
        hits(path, code, r"\bload\s*\(", "load in a bootstrap rule (the bootstrap's files are boot(path, k); load is "
             "the engine's, load(path, module, grammar))", failures)
    hits(CHANGE, strip_rust(CHANGE.read_text()), r"\bfn body\b", "change::body (O1)", failures)


def b1(failures: list) -> None:
    """B1, operations on blocks (Operations §2–§4, §13). A twin is phases over tiles of independent items, a function of
    its pinned slots, the blocks its descriptor table reaches, and its params alone (Operations §3). Rust data
    structures inside a call are allowed for now (the user, 2026-11-25); nothing may outlive a call or come from
    outside it. The twins' crate forbids unsafe, depends on blocks alone, and names no static, thread-local,
    Once/Lazy cell, lock or atomic, no thread's scratch, and no file, environment, process, network or time. Its
    clippy.toml disallows the same by path. A twin writes blocks' structures with blocks' own code: "the twins depend
    on blocks, and a slot is the pinned block, so a twin uses blocks' layouts and freeze in place" (the rebuild plan,
    B1). An operation's shape is phases on the six axes, with no enum of shapes
    ("No enum of shapes"); a version other than the reference is registered only with a check against it."""
    lib = TWINS / "src/lib.rs"
    if "#![forbid(unsafe_code)]" not in lib.read_text():
        failures.append(f"{lib.relative_to(ROOT)}: the twins' crate lacks #![forbid(unsafe_code)]")
    if not (TWINS / "clippy.toml").exists():
        failures.append(f"{TWINS.relative_to(ROOT)}: the twins' crate lacks its clippy.toml")
    manifest = (TWINS / "Cargo.toml").read_text()
    deps = manifest.split("[dependencies]", 1)[1] if "[dependencies]" in manifest else ""
    deps = re.split(r"^\[", deps, flags=re.M)[0]
    for name in re.findall(r"^\s*([\w-]+)\s*=", deps, flags=re.M):
        if name != "rerail-blocks":
            failures.append(f"{(TWINS / 'Cargo.toml').relative_to(ROOT)}: the twins' crate depends on {name} (blocks alone)")
    forbidden = [
        (r"\bstatic\b", "a static (state outliving a call)"),
        (r"thread_local|\bOnce\w*|\bLazy\w*|lazy_static|once_cell", "a thread-local or a once/lazy cell (state outliving a call)"),
        (r"\bMutex\b|\bRwLock\b|\bCondvar\b|\bAtomic\w*", "a lock or an atomic (shared state)"),
        (r"scratch::with\b", "a thread's scratch (it outlives the call: make a Scratch of the call's own)"),
        (r"std::(fs|env|net|process|time)\b|\bInstant\b|\bSystemTime\b", "the world (files, environment, processes, network, time)"),
        (r"\bunsafe\b", "unsafe"),
        (r"\bTwinFn\b", "a twin as one function (a twin is phases over tiles)"),
        (r"\bchange::", "blocks' change code (a twin's change is its own phases)"),
    ]
    for path in sorted((TWINS / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        code = re.sub(r"'static", "", code)
        for pattern, why in forbidden:
            hits(path, code, pattern, why, failures)
    op = CRATE / "src/op.rs"
    code = strip_rust(op.read_text())
    hits(op, code, r"\benum\s+Shape\b|\bShape::[A-Z]", "an enum of shapes (a shape is phases on the axes)", failures)
    for axis in ("Found", "Unit", "Inside", "Across", "Output", "Bound"):
        if not re.search(rf"\bpub enum {axis}\b", code):
            failures.append(f"{op.relative_to(ROOT)}: the {axis} axis is missing")
    if not re.search(r"pub fn register\(&mut self, v: Version, checked: Checked\)", code):
        failures.append(f"{op.relative_to(ROOT)}: register takes the Checked its check gave")
    if not re.search(r"pub struct Checked \{\s*op:", code):
        failures.append(f"{op.relative_to(ROOT)}: Checked's fields are private (only check makes one)")
    if "pub const MAX_SLOTS: usize = 30;" not in code:
        failures.append(f"{op.relative_to(ROOT)}: an operation binds at most 30 slots")


def b2(failures: list) -> None:
    """B2, the program trace (Operations §7, §10; P§11–§12): one bistate definition (0678's `Bistate`, in blocks); a
    trace with two states (the user, 2026-11-25), so no frontier API (`at`, `between`, `fork`, `pin`); each layer a
    frozen block of its own that refers to the one below by block id, read through a launch's descriptor table; the
    host holds handles only (no rows, no chain of its own, no lock); `consolidate` and `merge` phases in the twins'
    crate."""
    crates = ROOT / "native/crates"
    defs = []
    for path in sorted(crates.rglob("*.rs")):
        if "/target/" in str(path):
            continue
        text = path.read_text(errors="replace")
        if re.search(r"\bstruct\s+Bistate\b", text):
            defs.append(path.relative_to(ROOT))
    if [str(d) for d in defs] != ["native/crates/blocks/src/bistate.rs"]:
        failures.append(f"one Bistate, in blocks' bistate.rs: found {', '.join(map(str, defs)) or 'none'}")
    if (crates / "blocks/src/trace.rs").exists():
        failures.append("native/crates/blocks/src/trace.rs: a trace in one block (each layer is a block of its own)")
    layer = crates / "blocks/src/layer.rs"
    host = [layer, CRATE / "src/trace.rs", CRATE / "src/dir.rs"]
    for path in host:
        code = strip_rust(path.read_text())
        hits(path, code, r"\bMutex\b|\bRwLock\b", "a lock (claims are atomic; a block has one writer)", failures)
        hits(path, code, r"\bpub fn\s+(at|between|fork|pin|frontier)\b|\bstruct\s+Frontier\b", "a frontier API (a trace has two states)", failures)
    for path in [layer, crates / "blocks/src/bistate.rs", TWINS / "src/trace.rs"]:
        hits(path, strip_rust(path.read_text()), r"\bMAX_RUNS\b", "a bound on the layers a read or a merge follows", failures)
    code = strip_rust(layer.read_text())
    hits(layer, code, r"pub key: i64", "a partition's key in 64 bits (a module's or a document's id is 32)", failures)
    for kind in ("IX_FENCES", "IX_PRINTS", "IX_HASH", "IX_BLOOM"):
        if kind not in code:
            failures.append(f"{layer.relative_to(ROOT)}: the index kind {kind} (P§11.2) is missing")
    # The rebuild plan, B2: "Version and Bistate are its View generalized from a base and one delta to n layers ...
    # as one definition". A trace's state is relation::View (View::chain), reading its layers through the descriptor
    # table and holding no list of them; there is no second state type beside it.
    rel = crates / "blocks/src/relation.rs"
    rcode = strip_rust(rel.read_text())
    view = re.search(r"pub struct View<'a> \{(.*?)\}", rcode, flags=re.S)
    if not view or "Vec" in view.group(1) or "far: Option<&'a crate::gens::Table<'a>>" not in view.group(1):
        failures.append(f"{rel.relative_to(ROOT)}: View reads a trace's layers through the descriptor table, holding no list")
    if "pub fn chain(table: &'a Table<'a>, head: Option<Id>, arity: usize)" not in code:
        failures.append(f"{layer.relative_to(ROOT)}: a trace's state is View::chain")
    for path in [layer, crates / "blocks/src/bistate.rs", rel]:
        hits(path, strip_rust(path.read_text()), r"\bstruct\s+Chain\b|\bimpl\b[^{;]*\b(Read|States)\s+for\s+Chain\b", "a second state type beside relation::View (B2: one definition)", failures)
    trace = CRATE / "src/trace.rs"
    code = strip_rust(trace.read_text())
    hits(trace, code, r"\bVec<\s*(Layer|Chain|BlockId)\b|\bVec<Vec<i64>>\s*[,;)]", "the host holding layers or rows", failures)
    tstruct = re.search(r"pub struct Trace \{(.*?)\}", code, flags=re.S)
    if not tstruct or re.sub(r"\s+", " ", tstruct.group(1)).strip() != "shape: ShapeW, head: Option<BlockId>, roll: RollUp, cache: Option<BlockId>,":
        failures.append(f"{trace.relative_to(ROOT)}: a Trace is its shape, its head's block id, its roll-up policy and its cache header's block id")
    # O5: the geometric roll-up runs from commit, by the count consolidate's seal reads from the layers' headers.
    twins = (CRATE.parent / "delta_twins/src/trace.rs").read_text()
    if "fn due_to_merge(" not in twins or "due_to_merge(c.table" not in twins:
        failures.append("the twins' trace.rs: consolidate's seal counts the layers due to merge (due_to_merge)")
    if not re.search(r"RollUp::Geometric && due >= 2 \{\s*return self\.roll_up\(", code):
        failures.append(f"{trace.relative_to(ROOT)}: commit runs the geometric roll-up")
    if not re.search(r"head: None, roll: RollUp::Geometric", code):
        failures.append(f"{trace.relative_to(ROOT)}: a new trace rolls up geometrically")
    if "dir.refer(out" not in code:
        failures.append(f"{trace.relative_to(ROOT)}: a new layer refers to the one below by block id")
    for path in sorted(list((CRATE / "src").rglob("*.rs")) + list((TWINS / "src").rglob("*.rs"))):
        hits(path, strip_rust(path.read_text()), r"\bstruct\s+(Bistate|Frontier|Layer|Chain)\b", "a second trace or bistate (blocks holds the one)", failures)
    twins = (TWINS / "src/trace.rs").read_text()
    for name in ("pipeline!(CONSOLIDATE, Cons);", "pipeline!(MERGE, Merge);"):
        if name not in twins:
            failures.append(f"the twins' trace.rs lacks {name}")
    # P§12: "A layer is one blocks::relation::Relation"; the rebuild plan, B2: "built on blocks' relation and 0678's
    # bistate, as one definition". The layer's rows are a Relation continuing the one below, frozen where it lies; its
    # statistics are blocks' sketch (O§10), no second sketch; a permutation is packed's (P§12).
    code = strip_rust(layer.read_text())
    for want, why in (
        (r"pub rows: &'a Relation", "a layer's rows are a Relation"),
        (r"pub parts: &'a Relation", "a layer's partitions' change is a Relation"),
        (r"Sketch::decode", "a layer's statistics are blocks' sketch, read back from its kept form"),
        # A permutation is packed's (`packed::PIndex`): an index of a Packed or Dict layer alone, read in place.
        (r"IndexSpec::Perm\(key\) => \{\s*if !matches!\(self\.encoding, Encoding::Packed \| Encoding::Dict\)", "a Perm index of a Packed or Dict layer alone (packed's permutation, P§12)"),
        # An encoded layer's rows are blocks' own encodings, read in place through one reader (P§12).
        (r"pub own: OwnRows<'a>", "a layer's own rows read where they lie, in its encoding (OwnRows)"),
        (r"crate::packed::Packed", "a packed layer's rows are packed::Packed"),
        (r"crate::ranged::Columns", "a ranged layer's rows are ranged::Columns"),
    ):
        if not re.search(want, code):
            failures.append(f"{layer.relative_to(ROOT)}: {why}")
    hits(layer, code, r"\bSketchW\b|\bAMS\b|\bstruct\s+PartW\b|\bpflips\b", "a second sketch or a copy of a layer's kills", failures)
    twins_code = strip_rust(twins)
    for want, why in (
        ("Relation::try_continue_far(", "the layer made as a Relation continuing the one below"),
        ("layer_write::open(", "the layer's freeze laid out where it lies (blocks' layer writing)"),
        ("layer_write::index(", "the layer's hashed indexes, an index an item"),
        ("layer_write::seal(", "the layer frozen where it lies"),
        ("add_order(", "an Order index added to the relation (P§11.2)"),
        ("layer_write::sketch(", "the layer's sketches kept in its relation, a column an item"),
    ):
        if want not in twins_code:
            failures.append(f"the twins' trace.rs: {why} ({want} missing)")
    # O§2: the descriptor table names "per block id it can reach (its slots, and what they refer to, transitively)".
    dir_code = strip_rust((CRATE / "src/dir.rs").read_text())
    hits(CRATE / "src/dir.rs", dir_code, r"filter\(\|id\| slots\.iter\(\)\.all", "a descriptor table without the launch's slots (O§2)", failures)
    # B§7: "a crate of its own that depends on blocks and rr_syntax alone" (with its twins, which depend on blocks
    # alone), in its code as in its manifest: it names no other crate (its tests may name the device layer's programs,
    # a dev-dependency), and blocks re-exports no crate for it to reach another through.
    for path in sorted((CRATE / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        allowed = {"rerail_blocks", "rerail_rr_syntax", "rerail_delta_twins", "rerail_delta_exec"}
        if path.name.endswith("tests.rs"):
            allowed.add("rerail_gpu")
        for name in sorted(set(re.findall(r"\b(rerail_[a-z0-9_]+)\b", code)) - allowed):
            failures.append(f"{path.relative_to(ROOT)}: names {name} (B§7: blocks and rr_syntax alone)")
    blocks_lib = crates / "blocks/src/lib.rs"
    hits(blocks_lib, strip_rust(blocks_lib.read_text()), r"pub use rerail_device\b", "blocks re-exporting the device layer (a way around B§7)", failures)
    # The B2 checks the plan asks for, kept as tests: a restriction reads only its partition's rows, counted where the
    # reads are made; is_identity reads a layer's counts only.
    tests = (CRATE / "src/trace_tests.rs").read_text()
    for name in ("fn a_restriction_reads_only_its_partitions_rows", "fn is_identity_reads_a_layers_counts_only", "fn a_kill_search_reads_a_search_a_layer"):
        if name not in tests:
            failures.append(f"delta_exec's trace tests lack {name}")
    # P§: restrict is "a range of the block, by the partition's index": a partition's rows and kills are read from
    # parts, not found by a search of the layer's rows.
    layer = crates / "blocks/src/layer.rs"
    lcode = strip_rust(layer.read_text())
    if "vec![crate::rows::I32, 8, 4, 4, 4, 4]" not in lcode:
        failures.append(f"{layer.relative_to(ROOT)}: parts is (part, digest, added, gone, first, kills_at), the partition index")
    hits(layer, lcode, r"leading\(self\.(rows|gone)\b", "a partition found by a search of the layer's rows or kills, not its index", failures)
    # A layer's kills are kept once: its partition index's `gone(part, id)` (P§: "a range of the block, by the
    # partition's index"), read by partition for liveness, scans and flips alike. Its rows' relation records no change
    # (`layer_write` lays out none, `seal` refuses one, and nothing writes it); the layer holds three relations; `gone`
    # is (partition, id); the twins make exactly those three.
    lw = strip_rust((crates / "blocks/src/relation/layer_write.rs").read_text())
    if "kills_mut" in lw or "rel.changed.as_slice().is_empty()" not in lw:
        failures.append("blocks/src/relation/layer_write.rs: a layer's change is laid out or written (its kills are gone's alone)")
    if "self.rows.changed" in lcode:
        failures.append(f"{layer.relative_to(ROOT)}: a layer's kills read from its rows' change, not gone")
    twins_code = strip_rust((TWINS / "src/trace.rs").read_text())
    if re.search(r"\bchanged\b", twins_code) or re.search(r"\bks2?\b|W_KWIDTH|fn ksort|fn kplace", twins_code):
        failures.append("the twins' trace.rs writes or sorts a second copy of the kills (they are gone's alone)")
    m = re.search(r"pub struct Layer<'a> \{(.*?)\}", lcode, re.S)
    fields = re.findall(r"pub (\w+):", m.group(1)) if m else []
    # `own` is the reader of its rows where they lie (its relation's, or its encoded form's), not a store.
    if fields != ["bytes", "head", "rows", "parts", "gone", "own"]:
        failures.append(f"{layer.relative_to(ROOT)}: a layer holds {fields}; its rows, parts and gone alone, and its rows' reader")
    if "Spec { widths: vec![crate::rows::I32, 4], sort: Some(0)" not in lcode:
        failures.append(f"{layer.relative_to(ROOT)}: gone is (partition, id), sorted on the partition")
    tcode = strip_rust((TWINS / "src/trace.rs").read_text())
    made = len(re.findall(r"Relation::try_create\(", tcode)), len(re.findall(r"Relation::try_continue_far\(", tcode))
    if made != (2, 1):
        failures.append(f"the twins' trace.rs makes {made[0]} relations and {made[1]} layers; a layer's rows, its parts and its gone alone")


def axes(failures: list) -> None:
    """Operations §4: an operation's shape is phases on the six axes, and its twin runs as its shape says. The executor
    runs by the axes (a fixed count is the launch's from its start; only units that share nothing, combine or make a
    round run in any order; a phase runs again exactly when its units relate by rounds). Here: the trace's pipeline is
    one item only in the phases that open, freeze and seal a layer, each declared a fixed count of one group, and every
    other phase's items are rows, chunks, pairs, partitions, columns or indexes."""
    op = strip_rust((CRATE / "src/op.rs").read_text())
    for want, why in (
        (r"p\.found == Found::Fixed", "the executor checks a fixed count against the launch's start"),
        (r"matches!\(phase\.across, Across::Nothing \| Across::Combine \| Across::Rounds\)", "the executor shuffles only units that may run in any order"),
        (r"t\.again\.is_some\(\) != \(p\.across == Across::Rounds\)", "fits ties a phase's finisher to rounds"),
    ):
        if not re.search(want, op):
            failures.append(f"native/crates/delta_exec/src/op.rs: {why}")
    twins = (TWINS / "src/trace.rs").read_text()
    one = set(re.findall(r'TwinPhase \{ name: "(\w+)", items: \|_\| 1,', twins))
    if one != {"open", "freeze", "seal"}:
        failures.append(f"the twins' trace.rs: one-item phases {sorted(one)}; only open, freeze and seal are one group")
    shape = (CRATE / "src/trace.rs").read_text()
    for name in sorted(one):
        if not re.search(rf'ph\("{name}", Fixed, Threadgroup,', shape):
            failures.append(f"native/crates/delta_exec/src/trace.rs: phase {name} is one item, declared otherwise")


def reach(failures: list) -> None:
    """The twins' rules reach the blocks code the twins call to write a layer (decision 0686): relation::layer_write,
    where the twins' clippy lints are denied (blocks' clippy.toml lists the twins' paths, and the two lists agree),
    and which names no static, thread-local, Once/Lazy cell, lock, thread's scratch, or the world. Its unsafe is
    blocks' layout writing. The old freeze of a layer, one call over every row, is gone."""
    lw = ROOT / "native/crates/blocks/src/relation/layer_write.rs"
    if not lw.exists():
        failures.append("native/crates/blocks/src/relation/layer_write.rs is missing")
        return
    text = lw.read_text()
    if "#![deny(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]" not in text:
        failures.append(f"{lw.relative_to(ROOT)}: the twins' clippy lints are not denied")
    code = re.sub(r"'static", "", strip_rust(text))
    for pattern, why in (
        (r"\bstatic\b", "a static (state outliving a call)"),
        (r"thread_local|\bOnce\w*|\bLazy\w*|lazy_static|once_cell", "a thread-local or a once/lazy cell"),
        (r"\bMutex\b|\bRwLock\b|\bCondvar\b", "a lock"),
        (r"scratch::with\b", "a thread's scratch"),
        (r"std::(fs|env|net|process|time)\b|\bInstant\b|\bSystemTime\b", "the world"),
    ):
        hits(lw, code, pattern, why, failures)
    def rules(p):
        return [l for l in p.read_text().splitlines() if l.strip() and not l.lstrip().startswith("#")]
    if rules(TWINS / "clippy.toml") != rules(ROOT / "native/crates/blocks/clippy.toml"):
        failures.append("native/crates/blocks/clippy.toml: its disallowed paths differ from the twins' clippy.toml")
    relp = ROOT / "native/crates/blocks/src/relation.rs"
    hits(relp, strip_rust(relp.read_text()), r"\bfn freeze_layer\b|\bfn set_sketches\b", "a layer's freeze as one call (the twins' phases make it)", failures)


def e1(failures: list) -> None:
    """E1 on a device (Operations §2): a launch's pins are handed to its fence, each a GPU use until it completes,
    nothing open between; a discrete domain reads the directory's replicas."""
    crates = ROOT / "native/crates"
    region = crates / "blocks/src/region.rs"
    rcode = strip_rust(region.read_text())
    for want, why in (
        ("pub fn to_fence(mut self, fence: &Arc<Fence>, op: &'static str, domain: u32)", "a slot's pin handed to a fence"),
        ("handoff: Option<(bool, u64)>", "the CPU use ending as the GPU's begins, under one lock"),
        ("fn device_address(&self) -> Option<u64>", "a replica's address, for a device's table"),
    ):
        if want not in rcode:
            failures.append(f"{region.relative_to(ROOT)}: {why}")
    dir_rs = CRATE / "src/dir.rs"
    dcode = strip_rust(dir_rs.read_text())
    for want, why in (
        ("pub fn to_fence(self, fence: &Arc<Fence>, op: &'static str, domain: u32)", "a launch handed to its fence"),
        ("pub fn replicate(&mut self, id: BlockId, domain: u32", "replicas by the directory (0440 §7.2)"),
    ):
        if want not in dcode:
            failures.append(f"{dir_rs.relative_to(ROOT)}: {why}")
    if "pub fn device_rows_in(" not in strip_rust((crates / "blocks/src/gens.rs").read_text()):
        failures.append("blocks/src/gens.rs: a launch's table rows in a discrete domain, by its replicas")
    tests = strip_rust((CRATE / "src/op_tests.rs").read_text())
    if "fn a_launch_on_a_device_holds_its_blocks_until_its_fence_completes" not in tests:
        failures.append("delta_exec's op tests lack a_launch_on_a_device_holds_its_blocks_until_its_fence_completes")
    # The device side is rerail-delta-device's, above the executor (B§7: delta_exec compiles and launches nothing).
    device = crates / "delta_device"
    dtests = strip_rust((device / "src/tests.rs").read_text())
    dev = re.search(r"fn a_kernel_follows_references_through_the_device_table\(\) \{(.*?)\n\}", dtests, flags=re.S)
    if not dev or "l.to_fence(&fence" not in dev.group(1) or "dt.to_fence(&fence" not in dev.group(1):
        failures.append("delta_device's device test does not hand its launch to its fence")
    if "rerail-gpu" in (CRATE / "Cargo.toml").read_text():
        failures.append("delta_exec depends on rerail-gpu: its device is rerail-delta-device's (B§7)")
    # E3 on a device: the executor runs a UKL version through `Kernels`, its launch and its device table handed to
    # one fence; the device crate implements it, places blocks in a device domain (`region::DeviceCopy`, a backend's
    # Mirror) and its tests check a UKL version against its twin in both domains.
    op_code = strip_rust((CRATE / "src/op.rs").read_text())
    for want, why in (
        ("pub trait Kernels", "the executor's device interface"),
        ("launch.to_fence(&fence, sig.name, d.domain())", "a UKL version's launch handed to its fence"),
        ("Impl::Ukl { source, entry } =>", "the executor runs UKL versions"),
        ("pub fn check_on(", "a UKL version checked against the twin on a device"),
        ("pub fn run_on(", "a device's default version run"),
    ):
        if want not in op_code:
            failures.append(f"delta_exec/src/op.rs: {why}")
    dlib = strip_rust((device / "src/lib.rs").read_text())
    for want, why in (
        ("impl Kernels for Gpu", "the device implements the executor's Kernels"),
        ("gens::device_table_in(", "a device domain's table in its own domain"),
        ("region::DeviceCopy::new(", "replicas in a backend's memory"),
    ):
        if want not in dlib:
            failures.append(f"delta_device/src/lib.rs: {why}")
    if "impl Mirror for DeviceCopy" not in strip_rust((crates / "blocks/src/region.rs").read_text()):
        failures.append("blocks/src/region.rs: a backend's Mirror (DeviceCopy)")
    for t in ("fn a_ukl_version_is_checked_against_its_twin_on_the_device", "fn a_ukl_version_runs_on_replicas_in_a_device_domain"):
        if t not in dtests:
            failures.append(f"delta_device's tests lack {t}")


def main() -> int:
    failures: list = []
    for check in (m0, b1, b2, axes, reach, e1):
        check(failures)
    for f in failures:
        print(f)
    if failures:
        print(f"{len(failures)} shape check(s) failed (decision 0686, the rebuild plan)")
        return 1
    print("delta_exec shape: every check holds (M0, B1, B2, axes, reach, E1)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
