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
        (r"\b(trait|dyn)\s+World\b|std::fs::|File::open|read_to_string",
         "a file read of the crate's own (files come through the engine's file layer: the observer, placed reads)"),
    ]
    for path in sorted((CRATE / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        for pattern, why in forbidden:
            # A test may make the files it answers (write them, stamp them, remove them), and reads none.
            if path.name.endswith("_tests.rs") and "std::fs::" in pattern:
                pattern, why = (r"\b(trait|dyn)\s+World\b|fs::read|File::open|read_to_string|read_dir",
                                "a test's file read (a test writes the files the file layer answers, and reads none)")
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
    # Blocks, and the rr parser's tape reads in place (format 16's, with its language's table): what `intern` and
    # `k0_pass` read in their tape slot (B5, B6), the pair B§7 names for the executor's crate.
    for name in re.findall(r"^\s*([\w-]+)\s*=", deps, flags=re.M):
        if name not in ("rerail-blocks", "rerail-rr-syntax"):
            failures.append(f"{(TWINS / 'Cargo.toml').relative_to(ROOT)}: the twins' crate depends on {name} (blocks and the tape's reads alone)")
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
    if not tstruct or re.sub(r"\s+", " ", tstruct.group(1)).strip() != "shape: ShapeW, head: Option<BlockId>, roll: RollUp,":
        failures.append(f"{trace.relative_to(ROOT)}: a Trace is its shape, its head's block id and its roll-up policy")
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
        # B4: a document is answered through the engine's file layer (the observer, the slabs' placer), in `files`
        # alone.
        if path.name in ("files.rs", "files_tests.rs", "l1_tests.rs", "answer.rs", "answer_tests.rs"):
            allowed.add("rerail_observe")
        # The store's objects (sources and tapes by content, a tape by its grammar) and documents' places, kept as is
        # (*0686: Operations* §5; the user, 2026-11-26: "the objects store, documents' places and the journal"), in
        # `files` alone.
        if path.name == "files.rs":
            allowed |= {"rerail_store", "rerail_store_base"}
        # The residency's budget, slab size and drop: one definition with rerail_slabs (`rerail_accel::budget`).
        if path.name in ("residency.rs", "dir.rs"):
            allowed.add("rerail_accel")
        # The planner's learned correction (*0686: Operations* §8's `price`; 0354): the join model, given through `Cx`.
        if path.name in ("plan.rs", "plan_tests.rs", "contract.rs"):
            allowed.add("rerail_accel")
        # Its tests narrow the observer's racy window, through the context's switches, as the engine's do.
        if path.name == "files_tests.rs":
            allowed.add("rerail_context")
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


def bounds(failures: list) -> None:
    """Operations §4's bounds axis, each held by the executor: steps, a stack's depth and a sample reported by a twin
    through its meter, a slot's capacity refused, grown or the launch run again, turns bounded; the launch budget one
    ledger every launch charges (0601), not a phase's label. Bounds fit their axes when declared."""
    op = strip_rust((CRATE / "src/op.rs").read_text())
    for want, why in (
        (r"m\.steps = phase\.steps\(\)", "the executor gives a phase's steps bound to its meter"),
        (r"m\.depth = phase\.depth\(\)", "the executor gives a phase's stack bound to its meter"),
        (r"m\.sample = phase\.sample\(\)", "the executor gives a phase's sample to its meter"),
        (r"m\.bounded_room = phase\.capacity\(\)\.is_some\(\)", "the executor bounds a phase's room by its capacity bound"),
        (r"Some\(Over::Grow\) =>", "a capacity's grow is the executor's"),
        (r"Some\(Over::Rerun\) => return Ok\(Drove::Rerun", "a capacity's rerun is the executor's"),
        (r"dir\.charge\(sig\.name, phase\.name\)\?", "every phase's turn is a launch charged to the ledger (0601)"),
        (r"Self::fits_bounds\(sig, sig\.shape\)\?", "a signature's bounds fit its axes when declared"),
    ):
        if not re.search(want, op):
            failures.append(f"native/crates/delta_exec/src/op.rs: {why}")
    if re.search(r"\bLaunchBudget\b", op):
        failures.append("native/crates/delta_exec/src/op.rs: the launch budget is the ledger's (0601), not a phase's bound")
    shape = (CRATE / "src/trace.rs").read_text()
    if not re.search(r'ph\("kills",[^)]*KILL_STEPS\)', shape):
        failures.append("native/crates/delta_exec/src/trace.rs: the kill search bounds its steps, a layer a step")
    if not re.search(r"c\.steps\(depth\)", strip_rust((TWINS / "src/trace.rs").read_text())):
        failures.append("the twins' trace.rs: the kill search reports its steps")


def cache(failures: list) -> None:
    """P§11.3 and Operations §6.1: an operator's cache is mutable blocks, one state mutated in place, never frozen,
    with no generations; not a trace's layers. Its operations are real (not test-only), and `order` grows it."""
    layer = strip_rust((ROOT / "native/crates/blocks/src/layer.rs").read_text())
    if re.search(r"Retention::Cache\s*=>\s*RETAIN_CACHE|\bRETAIN_CACHE\b|fn is_cache", layer):
        failures.append("native/crates/blocks/src/layer.rs: a cache stored as a trace's layers (P§11.3: one block, mutated)")
    lib = (TWINS / "src/lib.rs").read_text()
    if re.search(r"#\[cfg\([^\]]*\)\]\s*pub mod cache;", lib):
        failures.append("the twins' lib.rs: the cache's twins are test-only")
    host = (CRATE / "src/cache.rs").read_text()
    if not re.search(r'one\("order", [^)]*Bound::Capacity\(Over::Grow\)', host):
        failures.append("native/crates/delta_exec/src/cache.rs: order grows the cache within its reservation")
    if re.search(r"\bdir\.freeze\(|\.refer\(", host):
        failures.append("native/crates/delta_exec/src/cache.rs: a cache block frozen or referring: it has no generations")
    trace = strip_rust((CRATE / "src/trace.rs").read_text())
    if re.search(r"cache", trace):
        failures.append("native/crates/delta_exec/src/trace.rs: a trace holds a cache (P§11.3: a cache is not a trace)")


def contract(failures: list) -> None:
    """B3: one operator contract (P§2, P§12 item 7). `DeltaOp` has P§'s ports: a bistate per input, the demand on its
    outputs, its cache, a writer per output and its asks of its inputs, returning its work; and its plain function.
    The reference is written once (run both sides, diff). The harness checks L1 to L5, and its tests hold a good toy
    and four that each break one law. An operator's cache is the operator cache's blocks, never a trace."""
    p = CRATE / "src/contract.rs"
    if not p.exists():
        failures.append("native/crates/delta_exec/src/contract.rs is missing (B3)")
        return
    text = strip_rust(p.read_text())
    if not re.search(r"type Ports: PortSet;\s*(///[^\n]*\n\s*)*type Cache: CacheBody;", text):
        failures.append("contract.rs: DeltaOp has not P§12's associated types (type Ports; type Cache)")
    if not re.search(r"fn delta\(&self, cx: &mut Cx<'_>, ins: &In<'_, Self::Ports>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<Self::Cache>, out: &mut Out<Self::Ports>, asks: &mut Asks<Self::Ports>\) -> Result<Work, String>", text):
        failures.append("contract.rs: DeltaOp::delta is not P§12's (ins, want, cache, out, asks) -> Work, typed by its ports and cache")
    if not re.search(r"fn run\(&self, cx: &mut Cx<'_>, ins: &At<'_, Self::Ports>, out: &mut Out<Self::Ports>\)", text):
        failures.append("contract.rs: DeltaOp::run, the plain function, is not P§12's (ins at a state, out)")
    if re.search(r"pub struct (In|At)<'t, P> \{[^}]*\bpub ports", text) or re.search(r"pub fn (block|kept|writer|room)\(&(mut )?self,[^)]*port: usize", text):
        failures.append("contract.rs: a port read by a bare index, not its port set's handle")
    if not re.search(r"pub fn reference<O: DeltaOp>", text):
        failures.append("contract.rs: the generic reference is not written once")
    raw = p.read_text()
    for law in ("L1", "L2", "L3", "L4", "L5", "D1"):
        if f'fail("{law}"' not in raw:
            failures.append(f"contract.rs: the harness does not check {law}")
    if "Retention::Cache" in text or not re.search(r"pub struct Instance \{\s*pub outs: Vec<Trace>,\s*pub cache: OpCache,\s*pub want: Trace,\s*pub asks: Vec<Option<Trace>>,\s*\}", text):
        failures.append("contract.rs: an operator's cache is not the operator cache's block (OpCache)")
    tests = (CRATE / "src/contract_tests.rs").read_text()
    for flaw, law in (("Kills", "L1"), ("Cold", "L2"), ("Busy", "L4"), ("Cache", "L5")):
        if f'fails_on(Flaw::{flaw}, "{law}")' not in tests:
            failures.append(f"contract_tests.rs: no toy built to break {law} ({flaw})")


def b4(failures: list) -> None:
    """B4: a document through the engine's file layer. The observer's digest step answers a path; a changed file is read
    once by the placed read, into the slabs' placer's home; the rr parser writes the tape into a tape block's room; the
    document is a blob named by its content; `Text` holds no String."""
    blob = ROOT / "native/crates/blocks/src/blob.rs"
    if not blob.exists() or not re.search(r"pub trait Blobs \{", blob.read_text()) or not all(f"pub struct {n}" in blob.read_text() for n in ("BlockId", "BlobId", "Placed")):
        failures.append("blocks/src/blob.rs: P§12 item 1's Blobs (BlockId, BlobId, Placed, the trait) not in the blocks crate")
    for f in sorted((CRATE / "src").glob("*.rs")):
        if re.search(r"pub struct (BlockId|BlobId|Placed)\b", f.read_text()):
            failures.append(f"{f.name}: a second definition of the blocks crate's blob names")
    p = CRATE / "src/files.rs"
    if not p.exists():
        failures.append("native/crates/delta_exec/src/files.rs is missing (B4)")
        return
    text = strip_rust(p.read_text())
    for pattern, why in ((r"fn place_in\(", "the placer: a room of the document's tape block"),
                         (r"kind::TAPE", "a tape block"), (r"blobs\.house\(", "a blob named by its content"),
                         (r"answer_load\(reg, &self\.ops", "the world's documents through the answer_load operation"),
                         (r"parse_sources\(reg, &self\.ops", "the world's new contents through the parse operation"),
                         (r"fn doc_of\((?:(?!\n\}\n).)*parse_sources\(", "a document in hand through the parse operation")):
        if not re.search(pattern, text, re.S):
            failures.append(f"files.rs: no {why}")
    # Δanswer as operations (*0686: Operations* §10): answer_load (the file layer as it is) and parse (the rr parser),
    # each declared with its slots and registered, as host versions, the one place the observer and parser are called.
    a = CRATE / "src/answer.rs"
    atext = strip_rust(a.read_text()) if a.exists() else ""
    for pattern, why in ((r"pub static ANSWER_LOAD: Signature", "answer_load declared as an operation"),
                         (r"pub static PARSE: Signature", "parse declared as an operation"),
                         (r"Version::host\(load,", "answer_load's host version registered"),
                         (r"Version::host\(parse,", "parse's host version registered"),
                         (r"\.digest_step\(", "the observer's digest step"), (r"\.digest_read\(", "the read recorded"),
                         (r"\.read_placed\(&place\)", "the placed read, through a placer"),
                         (r"tape_in_deep\(src, Arc::new\(InBlock\(", "the tape written by the parser into a room of the slab"),
                         (r"c\.depth\(deepest\)", "the parser's stack depth metered")):
        if not re.search(pattern, atext):
            failures.append(f"answer.rs: no {why}")
    for f in sorted((CRATE / "src").glob("*.rs")):
        if f.name not in ("answer.rs", "answer_tests.rs") and not f.name.endswith("tests.rs"):
            hits(f, strip_rust(f.read_text()), r"\.digest_step\(|\.read_placed\(|\btape_in(_deep)?\(|Text::parse_in\(", "the file layer or the parser called outside Δanswer's operations", failures)
    tape = strip_rust((CRATE / "src/tape.rs").read_text())
    m = re.search(r"pub struct Text \{([^}]*)\}", tape)
    if not m or "String" in m.group(1):
        failures.append("tape.rs: Text holds a String (B4: its source where the file layer placed it)")
    if (ROOT / "native/crates/lang/src/code_rule_dsl/tape/placed.rs").read_text().count("fn place(") > 1:
        failures.append("lang's placed.rs: a placer of its own beside the slabs' (one definition, rerail_slabs::placed)")


def b5(failures: list) -> None:
    """B5: symbols. One operation writes the atoms, `intern` (its twin, the one caller of `append_in`); the views read
    a document's symbols block (`intern`'s node → symbol facts) and never intern; the walk reads them too."""
    for path in sorted((CRATE / "src").rglob("*.rs")) + sorted((TWINS / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        if path != TWINS / "src/intern.rs":
            hits(path, code, r"\bappend_in\(|\bAtomsW\b|\.append\(", "an atoms write outside `intern` (B5: one writer)", failures)
        hits(path, code, r"\bSymbols\b|\.intern\(", "a symbol made inside a read (B5: `intern` writes them, reads read)", failures)
    if "pub static INTERN: Signature" not in (CRATE / "src/intern.rs").read_text():
        failures.append("intern.rs: intern is not an operation (B5)")


def b6(failures: list) -> None:
    """B6: L1. `k0_pass` is an operation of three phases (emit: count, scan, place), its twin the walk; L1 is a
    DeltaOp with B§4's ports, whose Rust beyond `k0_pass` and `drop_parts` is the wiring of its ports: it runs no
    other operation, writes no row itself, and never walks a tape on the host. A gone document is retracted from the
    rows L1 kept, read in its output traces (`drop_parts`, P§3), never by reading its old tape again with -1."""
    k0 = (CRATE / "src/k0.rs").read_text()
    if "pub static K0_PASS: Signature" not in k0:
        failures.append("k0.rs: k0_pass is not an operation (B6)")
    raw = (TWINS / "src/k0.rs").read_text()
    twin = strip_rust(raw)
    if not re.search(r'K0_PASS: &\[TwinPhase\] = &\[\s*TwinPhase \{ name: "count".*name: "scan".*name: "place"', raw, flags=re.S):
        failures.append("delta_twins/src/k0.rs: k0_pass's phases are not count, scan, place (emit)")
    hits(TWINS / "src/k0.rs", twin, r"\bRunsW\b|\bruns\w*\.push\(", "a row written but at its place (emit: count, scan, place)", failures)
    l1 = CRATE / "src/l1.rs"
    code = strip_rust(l1.read_text())
    for pattern, why in [
        (r"\brun_cpu\b|\brun_on\b|\breg\.(declare|run)", "an operation run but k0_pass (L1's Rust is its ports' wiring)"),
        (r"\bout\.writer\(|\bRunsW\b|\bpush_layer\b", "a row written by L1's Rust (k0_pass writes them)"),
        (r"\bwalk\(|\bstatement\(|\bReads\b|\.children\(", "a tape walked on the host (k0_pass's twin walks it)"),
    ]:
        hits(l1, code, pattern, why, failures)
    if 'name: "tape"' not in l1.read_text():
        failures.append("l1.rs: no tape port (B§4)")
    if 'globals: vec!["atoms"]' not in l1.read_text():
        failures.append("l1.rs: the atoms are not its facts in, a global port (B§4: facts in are program globals)")
    hits(l1, code, r"\bAtoms\b", "the atoms held beside L1's ports (they are its facts in)", failures)
    if not re.search(r"drop_parts\([^;]*out\.kept\(|out\.kept\([^;]*drop_parts\(", code, flags=re.S):
        failures.append("l1.rs: a gone document is not retracted from its kept rows (drop_parts over out.kept, P§3)")
    hits(l1, code, r"\bpass\([^;]*,\s*-1\s*,|k0_pass\([^;]*,\s*-1\s*,", "an old tape read again with -1 (retract the kept rows, P§3)", failures)
    if len(re.findall(r"\bk0_pass\(", code)) != 1:
        failures.append("l1.rs: k0_pass called at more than one place (one launch reads every changed tape)")
    if not re.search(r"pub fn k0_pass\([^)]*docs: &\[K0Doc\]", k0):
        failures.append("k0.rs: k0_pass does not take every document of the run (one launch)")
    if len(re.findall(r"\brun_cpu\(", strip_rust(k0))) != 1:
        failures.append("k0.rs: k0_pass launched at more than one place")
    files = (CRATE / "src/files.rs").read_text()
    if not re.search(r"pub struct BlobRows \{[^}]*\brows: Trace\b[^}]*\}", files) or re.search(r"pub struct BlobRows \{[^}]*HashMap", files):
        failures.append("files.rs: the blobs are not rows in a trace, as the store's objects are (P§12 item 1)")
    l1 = (CRATE / "src/l1.rs").read_text()
    k0h = (CRATE / "src/k0.rs").read_text()
    if not re.search(r"fn delta\([^)]*\bcache: &mut CacheW", l1) or "cache.write(cx)" not in l1 or "cache.body(cx," not in l1 or "type Cache = Sizes;" not in l1:
        failures.append("l1.rs: L1 keeps no cache, or reads none (Bootstrap, Figure 2: k0_pass rw L1's cache)")
    if not re.search(r'SlotSpec \{ name: "cache", mode: Mode::ReadWrite, kind: kind::CACHE \}', k0h):
        failures.append("k0.rs: k0_pass has no read-write slot for L1's cache")
    # The answerers (P§12 item 7): `answer(key, leaves) -> Answer`, a world reader's leaves into the run's layer; the
    # document set reads through all three.
    files = strip_rust((CRATE / "src/files.rs").read_text())
    answer_rs = strip_rust((ROOT / "native/crates/blocks/src/answer.rs").read_text())
    if not re.search(r"pub trait Answerer \{\s*const KIND: AnswerKind;\s*type Leaves;\s*fn answer\(&mut self, key: &\[i64\], leaves: &mut Self::Leaves\) -> Result<Answer, String>;", answer_rs):
        failures.append("blocks/src/answer.rs: the Answerer trait is not P§12's (answer(key, leaves) -> Answer)")
    if files.count("type Leaves = LeavesW;") != 3:
        failures.append("files.rs: an answerer whose leaves are not the run's layer of the world's leaves (LeavesW)")
    for n in ("Load", "Parse", "Intern"):
        if not re.search(rf"impl Answerer for {n}<'_>", files):
            failures.append(f"files.rs: {n} is not an answerer")
    if re.search(r"put_row\([^;]*&mut self\.leaves", files):
        failures.append("files.rs: a leaf committed on its own, not into the run's layer (LeavesW)")
    boot_code = strip_rust((CRATE / "src/boot.rs").read_text())
    for n in ("Load", "Parse", "Intern"):
        if not re.search(rf"\({n} \{{[^\n]*?\}}\)\.answer\(", boot_code):
            failures.append(f"boot.rs: the document set does not read through the {n} answerer")
    # Demand (P§7.1): L1 asks of its tape port (its only rows written in Rust: the demand on its input), restricts its
    # work to the want, and the boot's run answers only what is asked.
    if "const ASKED: &'static [usize] = &[0];" not in l1 or "asks.writer(cx.dir, L1Ports::TAPE)" not in l1 or "want_rows(" not in l1:
        failures.append("l1.rs: L1 neither restricts to its want nor asks of its tape port (P§7.1)")
    boot = (CRATE / "src/boot.rs").read_text()
    if "inst.asked(" not in boot or "docs.forget(" not in boot or "inst.set_want(" not in boot:
        failures.append("boot.rs: the run's demand does not reach the world as reads (P§7.1)")
    if "Asks::none()" in strip_rust((CRATE / "src/contract.rs").read_text()).split("pub fn none()")[-1]:
        failures.append("contract.rs: a driver runs an operator with no asks (P§7.1: demand out is a port)")
    if re.search(r"for drop_at in \[", (CRATE / "src/l1_tests.rs").read_text()):
        failures.append("l1_tests.rs: L5's cache dropped at fixed steps, not random ones")
    dirs = (CRATE / "src/dir.rs").read_text()
    if "dir.slab_room(kind::TAPE" not in files or re.search(r"make_(capped|placed)\(", files):
        failures.append("files.rs: a document's source and tape not rooms of a shared slab (the residency's, rerail_slabs)")
    if not re.search(r"alloc::block\(&need\)", dirs) or "Device::May" not in dirs:
        failures.append("dir.rs: a documents' slab not made by the block policy, as the device may read it (Need::SLAB, Device::May)")
    boot = (CRATE / "src/boot.rs").read_text()
    # Retirement is the residency's policy (rerail_slabs; the user, 2026-11-26), on P§11.4's leases: each slab held on
    # a lease its every use renews, aged out DROP_AFTER after its last; the least recently used slab retired early only
    # while the budget is short; never a run's own eager retirement, which threw the content hit away.
    residency = strip_rust((CRATE / "src/residency.rs").read_text())
    for want, why in (
        ("min_by_key(|(_, s)| s.clock)", "the least recently used slab retired"),
        ("held + bytes <= limit", "retired only while the budget is short"),
        ("rerail_accel::budget::resident()", "the residency's share of the budget, one definition with rerail_slabs"),
        ("rerail_accel::budget::DROP_AFTER", "a lease of DROP_AFTER"),
        ("now < h.used + lease", "a slab aged out when its lease lapsed (P§11.4: leased)"),
        ("s.used = Instant::now()", "a lease renewed by every use"),
        ("Arc::strong_count(&s.slab) == 1", "a slab a reader holds is not retired"),
    ):
        if want not in residency:
            failures.append(f"residency.rs: not the residency's policy: {why} ({want} missing)")
    if "self.residency.room_for(bytes)?" not in dirs:
        failures.append("dir.rs: a slab made without room under the residency's budget")
    if re.search(r"pub fn retire\(", files) or "docs.retire(" in boot:
        failures.append("files.rs/boot.rs: a run's own eager retirement (retirement is the residency's: rerail_slabs)")
    # P§12: the abstractions live in the blocks crate, beside the layer they wrap: Blobs and Answerer there, the
    # engine's homes and answerers implementing them.
    for f, what in (("blob.rs", "pub trait Blobs"), ("answer.rs", "pub trait Answerer")):
        if what not in (ROOT / "native/crates/blocks/src" / f).read_text():
            failures.append(f"blocks/src/{f}: {what} missing (P§12: the abstractions live in the blocks crate)")
    if re.search(r"pub (trait (Blobs|Answerer)|enum (Answer|AnswerKind))\b", files):
        failures.append("files.rs: a P§12 abstraction defined in delta_exec (it lives in the blocks crate)")
    if "forget_dead(" not in boot:
        failures.append("boot.rs: a run keeps the blobs of slabs the residency let go")
    if "fn executed(" in dirs or "fn executed(" in residency:
        failures.append("dir.rs/residency.rs: everything dropped at once after an execution (P§11.4: each slab leased)")
    if "fn the_residency_ages_out_a_slab_when_its_lease_lapses_and_a_use_renews_it" not in (CRATE / "src/files_tests.rs").read_text():
        failures.append("files_tests.rs: the lease's test is missing")
    interned = (TWINS / "src/intern.rs").read_text()
    if re.search(r"is_leaf\(n\)\s*\{\s*put\(", interned):
        failures.append("delta_twins/src/intern.rs: every leaf interned (intern names only identifiers and strings)")
    if not re.search(r"if find_in\(atoms, last,[^{]*\{\s*for n in language\(\)", interned):
        failures.append("delta_twins/src/intern.rs: the language's names not interned once, behind one find")
    boot = (CRATE / "src/boot.rs").read_text()
    if not re.search(r"pub fn run\([^)]*last: Option<&Pins>\) -> Result<\(Pins,", boot):
        failures.append("boot.rs: run does not take and return Pins (B§8)")
    if not re.search(r"pins of another state", boot):
        failures.append("boot.rs: a run from pins of another state is not refused (a trace has two states)")
    hits(CRATE / "src/boot.rs", strip_rust(boot), r"\bHashMap\b|\bBTreeMap\b", "the document set's rows held beside its trace (read them in place)", failures)
    if not re.search(r'DROP_PARTS: &\[TwinPhase\]', (TWINS / "src/drop.rs").read_text()):
        failures.append("delta_twins/src/drop.rs: drop_parts has no twin")
    for path in sorted((CRATE / "src").rglob("*.rs")):
        if path.name.endswith("_tests.rs") or path.name == "tests.rs":
            continue
        hits(path, strip_rust(path.read_text()), r"\bk0::walk\b|rerail_delta_twins::k0::(walk|statement)\b", "the walk called on the host, not through k0_pass", failures)


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


def kept(failures: list) -> None:
    """What lasts across processes (*Operations* §5, P§11.4, P§12 item 7; the user, 2026-11-26): the file layer kept
    as is (the store's objects, documents' places, the observer's journal), the leaves and the pure answers as kept
    traces (no map of the crate's own), the document key from the path's hash (0052 §3, one definition), the document
    set's path and grammar columns, indexed by path, and a later process starting where the last run left off."""
    src = CRATE / "src"
    files = strip_rust((src / "files.rs").read_text())
    for path in (src / "files.rs", src / "intern.rs", src / "boot.rs"):
        hits(path, strip_rust(path.read_text()), r"\bHashMap\b|\bBTreeMap\b", "a map where P§11.4 has a trace (the leaves, the answers by key)", failures)
    for want, why in (
        ("pub objects: ContentStore", "the store's objects (sources and tapes by content, a tape by its grammar)"),
        ("document_places(", "documents' places (0494): a later process reads a document where the segments hold it"),
        ("put_source_and_tape(", "a parsed document put in the store's objects"),
        ("i64::from(digest.is_some())", "every leaf recorded, misses too (D3)"),
        ("pub use rerail_blocks::answer::{Answer, AnswerKind, Answerer};", "P§12 item 7's answerer, the blocks crate's"),
        ("const KIND: AnswerKind = AnswerKind::World", "load, a world reader"),
        ("const KIND: AnswerKind = AnswerKind::Pure", "parse and intern, pure answers kept by key (D4)"),
        ("o.watch(&[root])", "the journal's cover (decision 0105)"),
        ("observe::store_beside(", "the observer's answers kept beside the content store, across processes"),
    ):
        if want not in files:
            failures.append(f"files.rs: {why} ({want} missing)")
    if "rerail_blocks::blob::doc_key(path)" not in files:
        failures.append("files.rs: a document's key not from its path's hash (0052 §3, rerail_blocks::blob::doc_key)")
    memo = (ROOT / "native/crates/corpus_memo/src/code_rule_dsl/logic/plan/corpus/memo.rs").read_text()
    if "rerail_blocks::blob::doc_key(path)" not in memo or "% SPAN" in memo:
        failures.append("corpus_memo memo.rs: the document key defined twice (one definition: rerail_blocks::blob::doc_key)")
    l1 = strip_rust((src / "l1.rs").read_text())
    if "IndexSpec::Hash(vec![TAPE_PATH as u16])" not in l1 or not re.search(r"pub fn tape_row\(d: i64, path: i64, grammar: i64", l1):
        failures.append("l1.rs: the document set's row without its path and grammar, or not indexed by path (P§11.4)")
    contract_code = strip_rust((src / "contract.rs").read_text())
    boot = strip_rust((src / "boot.rs").read_text())
    if "Directory::open_kept(kept" not in contract_code or "pins.keep(&mut bench.dir)" not in boot or "pub fn open(dir: &Directory)" not in boot:
        failures.append("contract.rs/boot.rs: a run's traces not kept, or a later process not opening them where its pins say")
    if "docs.tape.roll_up_run(" not in boot:
        failures.append("boot.rs: the document set never rolled up (its layers kept for ever)")
    if "pub fn open(dir: &mut Directory, path: &std::path::Path) -> Result<Atoms, String>" not in strip_rust((src / "sym.rs").read_text()):
        failures.append("sym.rs: the store's atoms not a file of their own (0401: ids that last across processes)")
    if "fn a_later_process_starts_where_the_last_run_left_off" not in (src / "l1_tests.rs").read_text():
        failures.append("l1_tests.rs: no check that a later process starts where the last run left off")


def b8(failures: list) -> None:
    """B§8's run (*Rebuild Plan* B§8, B§3, B§7; P§5; 0364; 0678 step 9): the rule body as three operations (`seed`,
    `step`, `head`) and `tally`; Δwave over counted derivations, the single-file rules at file grain through the
    weighted drop; Δfix over a levelled head (0364), its rechecks a candidate first step and a level filter; each
    stratum a module, passing the harness as a `DeltaOp`; the levels run over the export of the one before, K-form
    the program and the rest facts, until one exports no program; and the levels' pins kept across processes."""
    src = CRATE / "src"
    body = strip_rust((src / "body.rs").read_text())
    for op in ("SEED", "STEP", "HEAD"):
        if f"reg.declare(&{op})?" not in body:
            failures.append(f"body.rs: the rule body's {op.lower()} not an operation of the registry")
    if "reg.declare(&TALLY)?" not in strip_rust((src / "tally.rs").read_text()):
        failures.append("tally.rs: tally not an operation of the registry (0678 step 9)")
    phase = strip_rust((src / "phase.rs").read_text())
    for want, why in (
        ("impl DeltaOp for Stratum", "a stratum an operation the harness checks (B§10's laws)"),
        ("drop_counted(", "the single-file rules' changed files retracted by the weighted drop (P§3)"),
        ("tally::run(", "a non-recursive head netted against its counts (0678 step 9)"),
        ("s.levels = true;", "a recursive head's level column (0364)"),
        ("op: Op::Cmp(Cmp::Lt)", "a recheck's level filter (0364 §2.4)"),
        ("RELATIONS", "a level's program its K-form relations, the rest its facts (O6)"),
        ("pub fn exports(", "a level's export, which ends the loop when it holds no program"),
    ):
        if want not in phase:
            failures.append(f"phase.rs: {why} ({want} missing)")
    hits(src / "phase.rs", phase, r"\bfn\s+(fix|wave)_cold\b|\bnaive\b", "a stratum's change computed by a cold rerun beside Δwave or Δfix", failures)
    boot = strip_rust((src / "boot.rs").read_text())
    if "if !r.program {" not in boot or "MAX_LEVELS" not in boot:
        failures.append("boot.rs: the levels' loop does not end at the first level that exports no program (B§8)")
    if "levels.clear_from(" not in boot:
        failures.append("boot.rs: a level run from ∅ does not clear the levels after it")
    if "pub levels: Vec<LevelPins>" not in boot or "dir.refer_all(" not in boot or "pub fn refer_all(" not in (src / "dir.rs").read_text():
        failures.append("boot.rs: the levels' heads not in the pins, or the pins not referring to every head kept")
    tests = (src / "phase_tests.rs").read_text() + (src / "body_tests.rs").read_text()
    for t in (
        "fn a_wave_of_three_rules_over_facts_passes_the_harness",
        "fn a_transitive_closure_passes_the_harness_through_edits",
        "fn levels_stay_witnesses_through_edits",
        "fn an_anti_join_whose_matches_come_and_go_with_facts_and_filters_first",
        "fn each_refusal_of_b7_names_its_rule",
        "fn a_single_file_rule_over_tapes_passes_the_harness",
        "fn p2_run_as_a_level_over_l0s_tape_gives_p2",
        "fn a_level_loads_its_files_and_the_next_runs_their_program_through_edits",
        "fn an_edit_that_makes_a_stratum_recursive_runs_its_level_from_empty",
        "fn a_later_process_resumes_the_levels_where_the_last_run_left_them",
        "fn a_hand_written_plan_runs_a_join_on_each_side_and_on_a_change",
        "fn filters_facts_and_levels_in_a_hand_written_plan",
        "fn tally_moves_a_head_row_only_where_its_count_crosses_zero",
    ):
        if t not in tests:
            failures.append(f"B§8's tests lack {t}")


def planning(failures: list) -> None:
    """*0686: Operations* §8 and §13's planning track (P1–P4): lines and the budget (0122), price with the learned
    correction (0354) and the exact pairs (0203 §3), the search over subsets (0038) with its runners-up, choose with
    the memo and a seeded exploration (0203 §2, §6), emit_plan, check at its due points, the cut, trials and handoff
    (0206, 0207) and the limit again (0271), each an operation of the registry; the memo the stratum's operator cache;
    every chain of a stratum run through a planned round."""
    src = CRATE / "src"
    plan = strip_rust((src / "plan.rs").read_text())
    for name, value in (("SEARCH_WORK", "250.0"), ("GREEDY", "12"), ("RUNNERS_UP", "4"), ("SLOW_FACTOR", "8.0"), ("SLOW_ROWS", "100_000.0"), ("EXPLORE_RATE", "0.1"), ("MARGIN", "2.0"), ("EXPLORE_ROWS", "1_000_000.0"), ("TRIAL_ROWS", "256"), ("TRIAL_CUTOFF", "2.0"), ("TRIAL_SHARE", "0.1"), ("TRIAL_MARGIN", "1.25"), ("TRIAL_LEAST", "8")):
        if not re.search(rf"\bpub const {name}: \w+ = {re.escape(value)};", plan):
            failures.append(f"plan.rs: {name} is not {value} (Operations §8)")
    for op in ("LINES", "PRICE_OP", "SEARCH", "CHOOSE", "EMIT_PLAN", "CHECK", "STATS", "NOTE"):
        if f"d(&{op}," not in plan:
            failures.append(f"plan.rs: {op.lower()} not an operation of the registry (Operations §8)")
    for want, why in (
        ("model.fanouts(", "the learned correction asked in one batch (0354)"),
        ("fn limit_again(", "a handed-off order's limit (0271)"),
        ("body::emit_only(", "a trial launched over a sample of one line's tuples (0207)"),
        ("memo_twin::keep_line(", "the memo written into the operator's cache"),
    ):
        if want not in plan:
            failures.append(f"plan.rs: {why} ({want} missing)")
    # 0203 §2–§7: the memo's notes, the wider search, the step misses and the critical path, with their constants.
    for name, value in (("TRIED", "8"), ("RUN_WEIGHT", "0.5"), ("CRITICAL", "0.05"), ("CRITICAL_ROWS", "10_000.0"), ("CRITICAL_RATIO", "2.0"), ("WIDE_GREEDY", "16"), ("RANDOM_ORDERS", "48"), ("PROPOSALS", "6"), ("WIDE_PAYS", "100.0"), ("MISS_BINDINGS", "64.0")):
        if not re.search(rf"\bpub const {name}: \w+ = {re.escape(value)};", plan):
            failures.append(f"plan.rs: {name} is not {value} (0203)")
    for op in ("WEIGH", "CRITICAL_OP", "MARK"):
        if f"d(&{op}," not in plan:
            failures.append(f"plan.rs: {op.lower()} not an operation of the registry (0203 §4)")
    for want, why in (
        ("fn wide(", "the wider search (0203 §5)"),
        ("fn random_orders(", "the wider search's random connected orders (0203 §5)"),
        ("WIDE_PAYS * wide_cost(", "a wider search inline where its line's work pays for it (0203 §2)"),
        ("fn step_miss(", "the step misses (0203 §3a)"),
        ("note.merge_tried(", "a run's ratio merged as a geometric mean (0203 §6)"),
        ("n.matters()", "exploration only for lines that matter (0203 §7)"),
        ("!n.pending()", "a pending line searched, its kept order not taken (0203 §6)"),
        ("!on_path[ti] ||", "trials only on the critical path (0274)"),
        ("n.critical >= CRITICAL && !n.trialled", "a critical line trialled at its first check, once (0207 §2)"),
    ):
        if want not in plan:
            failures.append(f"plan.rs: {why} ({want} missing)")
    phase = strip_rust((src / "phase.rs").read_text())
    if "ops.plan.critical_path(" not in phase:
        failures.append("phase.rs: a level's run does not end with its critical path (0203 §4)")
    if "type Cache = crate::plan::Memo;" not in phase or "s.memo = Some(cache.write(cx)?);" not in phase:
        failures.append("phase.rs: the planner's memo is not the stratum's operator cache (P§11.3)")
    hits(src / "phase.rs", phase, r"\brun_plan\s*\(", "a stratum's chains run in their rules' order, unplanned", failures)
    body = strip_rust((src / "body.rs").read_text())
    if "C_TERM" not in body or "C_LIMIT" not in body:
        failures.append("body.rs: step has no trial's launch (one chain, a limit: 0207)")
    tests = (src / "plan_tests.rs").read_text()
    for t in (
        "fn a_chain_bound_again_in_its_own_order_is_itself_and_every_valid_order_answers_the_same",
        "fn the_search_takes_the_cheapest_order_and_keeps_the_runners_up",
        "fn a_round_plans_keeps_its_order_under_the_budget_and_a_dropped_memo_answers_the_same",
        "fn a_learned_correction_changes_the_order_chosen_not_the_answer",
        "fn exploration_is_a_seeded_draw_and_runs_a_runner_up",
        "fn a_slow_line_is_cut_trialled_and_handed_off_to_a_better_order",
        "fn a_handed_off_order_is_limited_again",
        "fn random_orders_are_distinct_valid_and_connected",
        "fn a_wider_search_is_exact_past_the_rounds_bound_and_proposes_distinct_orders",
        "fn step_misses_are_judged_past_eight_searches_of_work",
        "fn the_critical_path_marks_the_lines_of_its_heaviest_chain",
        "fn a_slow_line_on_the_critical_path_is_trialled_and_handed_off_to_a_better_order",
        "fn a_critical_line_is_trialled_once_at_its_first_check",
    ):
        if t not in tests:
            failures.append(f"the planning track's tests lack {t}")
    if "fn a_levels_run_marks_its_critical_lines" not in (src / "phase_tests.rs").read_text():
        failures.append("the test that a level's run marks its critical lines is missing")


def k1(failures: list) -> None:
    """B§10 step 7, the first K1 feature in rules: the binding lowerer in K0, one file a stratum (O4), K1's reader
    before it; no lowering in Rust; a level's documents the `load` facts of its facts in and of its program."""
    kernel = CRATE / "rerail" / "kernel"
    files = ("k1-0-numbers", "k1-1-read", "k1-2-shared", "k1-3-rank", "k1-4-rules", "k1-5-steps", "k1-6-binds", "k1-7-place",
             "k1-8-order", "k1-9-turn", "k1-10-emit")
    heads = {"k1-0-numbers": ("next",),
             "k1-1-read": ("head_of", "several", "alt", "group_alt", "split", "aux_alt", "scope", "under", "up", "lit_step", "lit_arg",
                           "expr", "paren", "op_step", "op_arg", "term_root", "in_term", "read_decl"),
             "k1-2-shared": ("onesplit", "ctx", "shared", "rep", "gt"), "k1-3-rank": ("succ", "least", "rank"),
             "k1-4-rules": ("inst", "read_rule", "read_arg", "in_rule", "read_step"),
             "k1-5-steps": ("in_atom", "mover", "below", "in_filter", "uses_var", "bound_before"), "k1-6-binds": ("binds", "waits"),
             "k1-7-place": ("place", "moved"), "k1-8-order": ("const_op", "at", "before", "twixt", "not_first"),
             "k1-9-turn": ("then", "turn", "placed"), "k1-10-emit": ("k_rule", "k_step", "k_arg", "k_var", "k_order", "k_decl")}
    for f in files:
        path = kernel / f"{f}.rerail"
        if not path.exists():
            failures.append(f"kernel/{f}.rerail missing (B§10 step 7, one file a stratum)")
            continue
        code = strip_rerail(path.read_text())
        for h in heads.get(f, ()):
            if not re.search(rf"^{h}\(", code, re.M):
                failures.append(f"kernel/{f}.rerail: no rule writes {h} (Steps, Strata, Roll-up §2)")
    for path in sorted((CRATE / "src").rglob("*.rs")):
        if path.name.endswith("_tests.rs"):
            continue
        hits(path, strip_rust(path.read_text()), r"\bbound_before\b|\bwaits\b|\bk1-\d", "the binding lowerer in Rust "
             "(it is rules, in kernel/)", failures)
    phase = strip_rust((CRATE / "src" / "phase.rs").read_text())
    if "r.head == load" not in phase or "self.load != Some(" not in phase:
        failures.append("phase.rs: a level answers no load facts of its program, or passes its asks on as facts out")
    if "self.caches.insert(m, inst.cache);" not in phase:
        failures.append("phase.rs: a level's module does not keep its operator cache, the planner's memo, across runs (Operations §6.1)")
    if "fn a_levels_memo_lasts_from_one_run_to_the_next" not in (CRATE / "src" / "phase_tests.rs").read_text():
        failures.append("the test that a level's memo lasts across runs is missing")
    if "fn a_k1_rule_lowered_by_p3_gives_the_rows_of_its_hand_ordered_k0_form" not in (CRATE / "src" / "phase_tests.rs").read_text():
        failures.append("B§10 step 7's test is missing")


def declared(failures: list) -> None:
    """Decision 0361 in the delta executor: relations declare their shapes as facts (`key`, `fanout`), read where they
    lie; a line they cover takes its default plan, unpriced and unsearched; run time checks they hold, and a line whose
    rows broke them is planned from statistics."""
    plan = strip_rust((CRATE / "src" / "plan.rs").read_text())
    body = strip_rust((CRATE / "src" / "body.rs").read_text())
    program = strip_rust((CRATE / "src" / "program.rs").read_text())
    for code, what in (("fn declared_order(", "the declared default plan"), ("const DECLARED: i64", "the declared line's status"),
                       ("min(sp.cap(mask))", "a probe's fan capped by its declarations"),
                       ("r.broken = true", "the run-time check of a declared plan"), ("note.broken |= r.broken", "a broken note")):
        if code not in plan:
            failures.append(f"plan.rs: {what} is missing (0361)")
    if "pub(crate) fn declared(&self, src: usize)" not in body or "Read::matching(after" not in body:
        failures.append("body.rs: the declarations are not read where they lie (0361, 0353)")
    if 'decls: crate::shapes::DECLS.map(' not in program:
        failures.append("program.rs: the engine does not know the declarations' relations (0361)")
    shapes = strip_rust((CRATE / "src" / "shapes.rs").read_text()) if (CRATE / "src" / "shapes.rs").exists() else ""
    if '"key", "fanout", "tree", "order"' not in (CRATE / "src" / "shapes.rs").read_text():
        failures.append("shapes.rs: the declarations are not key, fanout, tree and order (0361)")
    if "pub fn infer(" not in shapes or "fn closure(" not in shapes:
        failures.append("shapes.rs: no inference of derived relations' facts through their rules (0361)")
    phase = strip_rust((CRATE / "src" / "phase.rs").read_text())
    if "Work { broken" not in phase or "pub broken: Vec<(i64, u64)>" not in strip_rust((CRATE / "src" / "contract.rs").read_text()):
        failures.append("a broken declared fact is not named in the run's work (0361 item 3)")
    if "named: bool" not in phase or "known.wave" not in phase:
        failures.append("phase.rs: no named algorithm, by(rel, wave) (0361 item 4's override)")
    if "fn along_of(" not in phase or "return self.wave_along(" not in phase or ".with_facts(" not in phase:
        failures.append("phase.rs: no algorithm found from the facts: a recursion along a declared tree in one wave (0361 item 4; Operations, class 30)")
    if "crate::shapes::infer(" not in phase or ".with_inferred(inferred)" not in phase:
        failures.append("phase.rs: a level does not infer its strata's heads' facts, or a stratum does not read them (0361)")
    tests = (CRATE / "src" / "plan_tests.rs").read_text() + (CRATE / "src" / "phase_tests.rs").read_text()
    tests += (CRATE / "src" / "shapes_tests.rs").read_text() if (CRATE / "src" / "shapes_tests.rs").exists() else ""
    for t in ("fn a_rules_head_keeps_the_keys_its_body_determines", "fn a_fanout_carries_where_an_atoms_row_determines_the_heads_and_unions_keep_nothing",
              "fn a_derived_relations_inferred_key_gives_a_later_line_its_default_plan", "fn a_declared_key_or_fanout_bounds_a_probe",
              "fn a_declared_tree_and_order_give_their_keys_and_are_checked",
              "fn a_recursion_along_a_declared_tree_runs_in_one_wave_with_the_rounds_levels", "fn a_declared_line_takes_its_default_plan_and_a_broken_declaration_is_planned_from_statistics",
              "fn a_levels_declared_keys_give_its_lines_default_plans"):
        if t not in tests:
            failures.append(f"0361's test {t} is missing")


def main() -> int:
    failures: list = []
    for check in (m0, b1, b2, axes, bounds, cache, contract, b4, b5, b6, reach, e1, kept, b8, planning, k1, declared):
        check(failures)
    for f in failures:
        print(f)
    if failures:
        print(f"{len(failures)} shape check(s) failed (decision 0686, the rebuild plan)")
        return 1
    print("delta_exec shape: every check holds (M0, B1, B2, axes, bounds, cache, contract, B4, B5, B6, reach, E1, kept, B8, planning, K1, declared)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
