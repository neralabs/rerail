//! Device against CPU, over many generated strata: every kernel, operator and combinator the device's joins have
//! (joins by keys and scans, semi and anti joins, filters, lets, documents, sets, integer arithmetic, aggregates, text
//! tests by automaton and of bound patterns, the order and length of names, cats held, made and in heads, text
//! functions per row and per distinct name, chains of them, regex replacements, unions of rules, recursion), each
//! stratum run on the device and on the CPU, their rows (by name, for symbols) and their counts per step the same.
//!
//! Each case is a stratum of its own, so a refusal names its case. A case the device must run (`Case::must`) fails
//! when it is refused; the others (shapes the device refuses by design) need only the same rows. The summary prints
//! each family's cases, those the device ran, and the refusals' reasons.

use rusqlite::Connection;
use std::collections::BTreeMap;

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn sym(s: &str) -> String {
    format!("x:{}", hex(s))
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self, m: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % m.max(1)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.next(xs.len() as u64) as usize]
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next(100) < pct
    }
}

/// The relations the cases read: integers (duplicates, negatives, extremes, documents), an empty one, one of a row,
/// a graph, and names (empty, long, past ASCII, separators, cases), with a second relation of names some shared.
struct Fixture {
    db: Connection,
    _store: std::path::PathBuf,
}

const PARTS: [&str; 18] = ["foo", "Bar", "SRC", "src/lib", "a.b", "_", ".ts", "αβ", "x9", "TODO", "a_b", "/", "é", " ", "test", "Q", "0", "--"];

fn fixture() -> Fixture {
    let store = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&store));
    let db = Connection::open(&store).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
    let mut r = Rng(0x5eed);
    db.execute_batch(
        "CREATE TEMP TABLE ia (c0, c1); CREATE TEMP TABLE ib (c0, c1); CREATE TEMP TABLE ic (c0);
         CREATE TEMP TABLE iz (c0, c1); CREATE TEMP TABLE io (c0, c1); CREATE TEMP TABLE ie (c0, c1);
         CREATE TEMP TABLE dn (c0, c1); CREATE TEMP TABLE sn (c0, c1); CREATE TEMP TABLE sm (c0);
         CREATE TEMP TABLE sp (c0, c1);",
    )
    .unwrap();
    let ins2 = |t: &str, a: i64, b: i64| db.execute(&format!("INSERT INTO {t} VALUES (?1, ?2)"), [a, b]).unwrap();
    for _ in 0..500 {
        ins2("ia", r.next(60) as i64, r.next(200) as i64 - 50);
    }
    for (a, b) in [(i64::MAX, 1), (i64::MIN + 1, -1), (0, 0), (1, i64::MAX), (-7, i64::MIN + 1), (59, 0)] {
        ins2("ia", a, b);
    }
    for _ in 0..700 {
        ins2("ib", r.next(200) as i64 - 50, r.next(60) as i64);
    }
    for _ in 0..40 {
        db.execute("INSERT INTO ic VALUES (?1)", [r.next(60) as i64]).unwrap();
    }
    ins2("io", 3, 4);
    for v in 0..80 {
        ins2("ie", v, v + 1);
        if r.chance(25) {
            ins2("ie", v, r.next(80) as i64);
        }
    }
    for _ in 0..300 {
        ins2("dn", ((r.next(4) as i64) << 32) | r.next(50) as i64, r.next(50) as i64);
    }
    let mut names = Vec::new();
    for k in 0..500 {
        let len = 1 + r.next(if k % 40 == 0 { 40 } else { 5 });
        let name: String = if k % 97 == 3 { String::new() } else { (0..len).map(|_| *r.pick(&PARTS)).collect() };
        db.execute("INSERT INTO sn VALUES (?1, ?2)", rusqlite::params![name, k as i64]).unwrap();
        names.push(name);
    }
    for k in 0..60 {
        let name = if k % 2 == 0 { names[r.next(names.len() as u64) as usize].clone() } else { (0..1 + r.next(3)).map(|_| *r.pick(&PARTS)).collect() };
        db.execute("INSERT INTO sm VALUES (?1)", rusqlite::params![name]).unwrap();
    }
    for (k, p) in ["foo", "src", "", ".ts", "αβ", "a", "TODO", "_"].iter().enumerate() {
        db.execute("INSERT INTO sp VALUES (?1, ?2)", rusqlite::params![p, k as i64]).unwrap();
    }
    Fixture { db, _store: store }
}

fn src(t: &str, cols: &str, types: &str) -> String {
    format!("t:x:{}:{}:{types}", hex(&format!("temp.{t}")), hex(cols))
}

/// The fixture's relations as join sources.
struct Rels {
    ia: String,
    ib: String,
    ic: String,
    iz: String,
    io: String,
    ie: String,
    dn: String,
    sn: String,
    sm: String,
    sp: String,
}

fn rels() -> Rels {
    Rels {
        ia: src("ia", "c0, c1", "ii"),
        ib: src("ib", "c0, c1", "ii"),
        ic: src("ic", "c0", "i"),
        iz: src("iz", "c0, c1", "ii"),
        io: src("io", "c0, c1", "ii"),
        ie: src("ie", "c0, c1", "ii"),
        dn: src("dn", "c0, c1", "ii"),
        sn: src("sn", "c0, c1", "si"),
        sm: src("sm", "c0", "s"),
        sp: src("sp", "c0, c1", "si"),
    }
}

/// A stratum: its seed rules, its recursive rules (`join 0 0`), its head's types (two columns).
struct Case {
    family: &'static str,
    head: &'static str,
    seeds: Vec<String>,
    joins: Vec<String>,
    /// Whether the device must run it (a refusal fails the case).
    must: bool,
}

fn case(family: &'static str, head: &'static str, rule: String, must: bool) -> Case {
    Case { family, head, seeds: vec![rule], joins: Vec::new(), must }
}

type Rows = Vec<(String, String)>;

/// The stratum run once: its rows (by name for a symbol column), its counts per step, and the device's report.
fn run(fx: &Fixture, c: &Case, id: usize, device: bool) -> Result<(Rows, String, Option<super::Report>), String> {
    let db = &fx.db;
    db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
    let name = format!("par_{id}_{}", u8::from(device));
    let mut spec = vec![format!("name {name}")];
    if device {
        spec.push("device".into());
    }
    spec.push(format!("rel h0 {} empty", c.head));
    spec.extend(c.seeds.iter().map(|r| format!("seed 0 {}", hex(r))));
    spec.extend(c.joins.iter().map(|r| format!("join 0 0 {}", hex(r))));
    super::super::open(db, &spec.join("\n")).map_err(|e| format!("open: {e}"))?;
    let ran = super::super::run(db, &name, 0).map_err(|e| format!("run: {e}"));
    let report = super::take_report();
    let counts = super::super::step_counts(db, &name).map(|c| format!("{c:?}")).unwrap_or_default();
    super::super::close(db, &name).map_err(|e| format!("close: {e}"))?;
    ran?;
    let col = |k: usize| if c.head.as_bytes()[k] == b's' { format!("sym_text(c{k})") } else { format!("CAST(c{k} AS TEXT)") };
    let mut rows: Rows = db
        .prepare(&format!("SELECT {}, {} FROM temp.h0", col(0), col(1)))
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default())))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.sort_unstable();
    Ok((rows, counts, report))
}

#[derive(Default)]
struct Tally {
    cases: usize,
    on_device: usize,
    rows: usize,
    refused: BTreeMap<String, usize>,
}

/// Runs every case on the device and on the CPU; the failures (rows or counts that differ, a refused `must` case, an
/// error), and the summary by family.
fn check(fx: &Fixture, cases: &[Case]) -> (Vec<String>, BTreeMap<&'static str, Tally>) {
    let mut failures = Vec::new();
    let mut tally: BTreeMap<&'static str, Tally> = BTreeMap::new();
    for (id, c) in cases.iter().enumerate() {
        let t = tally.entry(c.family).or_default();
        t.cases += 1;
        let what = || format!("[{} #{id}] seeds {:?} joins {:?}", c.family, c.seeds, c.joins);
        let (dev, cpu) = (run(fx, c, id, true), run(fx, c, id, false));
        let ((drows, dcounts, report), (crows, ccounts, _)) = match (dev, cpu) {
            (Ok(d), Ok(c)) => (d, c),
            (Err(e), Ok(_)) => {
                failures.push(format!("{}: the device's run failed: {e}", what()));
                continue;
            }
            (Ok(_), Err(e)) => {
                failures.push(format!("{}: the CPU's run failed (the device's did not): {e}", what()));
                continue;
            }
            // Both refuse the spec alike (a shape the engine has no step for).
            (Err(e), Err(_)) => {
                let kind: String = e.chars().take(110).collect();
                *t.refused.entry(format!("(by both, as a spec) {kind}")).or_default() += 1;
                continue;
            }
        };
        let refused = report.as_ref().map_or(Some("no device report".to_owned()), |r| r.refused.clone());
        match &refused {
            None => t.on_device += 1,
            Some(why) => {
                // The reason's kind, without the rule's own names.
                let kind: String = why.split(':').next().unwrap_or(why).chars().take(110).collect();
                *t.refused.entry(kind).or_default() += 1;
                if c.must {
                    failures.push(format!("{}: refused by the device: {why}", what()));
                }
            }
        }
        t.rows += crows.len();
        if drows != crows {
            let only_d: Vec<_> = drows.iter().filter(|r| !crows.contains(r)).take(3).collect();
            let only_c: Vec<_> = crows.iter().filter(|r| !drows.contains(r)).take(3).collect();
            failures.push(format!("{}: rows differ: device {} rows, CPU {}; device alone {only_d:?}, CPU alone {only_c:?}", what(), drows.len(), crows.len()));
        } else if dcounts != ccounts && c.joins.is_empty() {
            // A recursive stratum's counts are its strategy's (the CPU walks a closure without counting rounds): its
            // rows are compared alone.
            failures.push(format!("{}: counts per step differ: device {dcounts}, CPU {ccounts}; report {report:?}", what()));
        }
    }
    (failures, tally)
}

fn report(name: &str, failures: &[String], tally: &BTreeMap<&'static str, Tally>) {
    eprintln!("== {name}: family, cases, run on the device, rows (the CPU's); refusals by reason");
    for (f, t) in tally {
        eprintln!("  {f:<22} {:>4} {:>4} {:>8}", t.cases, t.on_device, t.rows);
        for (why, n) in &t.refused {
            eprintln!("      {n:>4} refused: {why}");
        }
    }
    for f in failures.iter().take(25) {
        eprintln!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{name}: {} of the cases differ or were refused (first above)", failures.len());
}

/// Integer constants the filters and arithmetic meet: zero, small, negative, and the extremes.
const INTS: [i64; 10] = [0, 1, 3, 5, 40, -1, -50, 1 << 40, i64::MAX, i64::MIN + 1];

/// Joins, semi and anti joins, filters, lets, documents and sets, over every input shape (duplicates, an empty
/// relation, one row, keys of constants, a slot repeated).
fn relational(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let two = [&r.ia, &r.ib, &r.iz, &r.io, &r.ie, &r.dn];
    for a in two {
        for b in two {
            v.push(case("join", "ii", format!("pos {a} v0 v1\npos {b} v1 v2\nhead v0 v2"), true));
            v.push(case("join", "ii", format!("pos {a} v0 v1\npos {b} v0 v2\nhead v1 v2"), true));
            v.push(case("semi", "ii", format!("pos {a} v0 v1\npos {b} v1 _\nhead v0 v1"), true));
            v.push(case("semi", "ii", format!("pos {a} v0 v1\npos {b} v0 v1\nhead v0 v1"), true));
            v.push(case("anti", "ii", format!("pos {a} v0 v1\nneg {b} v1 _\nhead v0 v1"), true));
            v.push(case("anti", "ii", format!("pos {a} v0 v1\nneg {b} v0 v1\nhead v0 v1"), true));
        }
        v.push(case("join", "ii", format!("pos {a} v0 v0\nhead v0 v0"), true));
        v.push(case("join", "ii", format!("pos {a} v0 i:3\nhead v0 i:7"), true));
        v.push(case("join", "ii", format!("pos {a} i:1 v0\nhead v0 i:1"), true));
        v.push(case("anti", "ii", format!("pos {a} v0 v1\nneg {} v0\nhead v0 v1", r.ic), true));
        v.push(case("anti", "ii", format!("pos {a} v0 v1\nneg {} i:4\nhead v0 v1", r.ic), true));
        v.push(case("join", "ii", format!("pos {a} v0 v1\npos {} v1\npos {} v0 v2\nhead v2 v1", r.ic, r.ib), true));
        // A small cross product.
        v.push(case("join", "ii", format!("pos {} v0 v1\npos {} v2\nhead v0 v2", r.io, r.ic), true));
        for k in INTS {
            for op in ["lt", "le"] {
                v.push(case("filter", "ii", format!("pos {a} v0 v1\n{op} v1 i:{k}\nhead v0 v1"), true));
                v.push(case("filter", "ii", format!("pos {a} v0 v1\n{op} i:{k} v0\nhead v0 v1"), true));
            }
            v.push(case("filter", "ii", format!("pos {a} v0 v1\neq v1 i:{k}\nhead v0 v1"), true));
            v.push(case("filter", "ii", format!("pos {a} v0 v1\nne v0 i:{k}\nhead v0 v1"), true));
            v.push(case("let", "ii", format!("pos {a} v0 v1\neq v2 i:{k}\nhead v0 v2"), true));
        }
        for op in ["lt", "le", "eq", "ne"] {
            v.push(case("filter", "ii", format!("pos {a} v0 v1\n{op} v0 v1\nhead v0 v1"), true));
            v.push(case("filter", "ii", format!("pos {a} v0 v1\npos {} v1 v2\n{op} v2 v0\nhead v0 v2", r.ib), true));
        }
        v.push(case("let", "ii", format!("pos {a} v0 v1\neq v2 v1\neq v3 v2\nhead v3 v0"), true));
        v.push(case("set", "ii", format!("pos {a} v0 v1\nin v1 1 i:0 i:3 i:-1 i:40\nhead v0 v1"), true));
        v.push(case("set", "ii", format!("pos {a} v0 v1\nin v0 0 i:0 i:3 i:59\nhead v0 v1"), true));
        v.push(case("doc", "ii", format!("pos {a} v0 v1\ndocof v2 v0\nhead v2 v1"), true));
        v.push(case("doc", "ii", format!("pos {a} v0 v1\npos {} v2 v1\ndeq v0 v2\nhead v0 v2", r.dn), true));
        v.push(case("doc", "ii", format!("pos {a} v0 v1\npos {} v2 v1\ndne v0 v2\nhead v0 v2", r.dn), true));
    }
    v
}

/// Integer arithmetic (each operator of slots and constants: overflow and a zero divisor drop the row), then
/// filtered and in heads.
fn arithmetic(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let ops = ["rr_add", "rr_sub", "rr_mul", "rr_div", "mod", "rr_max", "rr_min"];
    for a in [&r.ia, &r.ib, &r.dn] {
        for op in ops {
            v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc {op} v2 v0 v1\nhead v0 v2"), true));
            v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc {op} v2 v1 v0\nlt v2 v0\nhead v1 v2"), true));
            for k in [0, 3, -1, i64::MAX, i64::MIN + 1] {
                v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc {op} v2 v1 i:{k}\nhead v0 v2"), true));
                v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc {op} v2 i:{k} v0\nhead v2 v1"), true));
            }
        }
        // Chains of them, and a value bound twice.
        v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc rr_add v2 v0 v1\nfunc rr_mul v3 v2 v2\nfunc mod v4 v3 i:7\nhead v0 v4"), true));
        v.push(case("arith", "ii", format!("pos {a} v0 v1\nfunc rr_sub v2 v0 v1\nfunc rr_div v3 v0 v2\nfunc rr_max v4 v3 i:-3\nhead v4 v1"), true));
    }
    v
}

/// Aggregates: count, sum, min and max, by keys and over the whole relation, bound and tested, of empty groups.
fn aggregates(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    for (outer, inner) in [(&r.ic, &r.ia), (&r.ic, &r.ib), (&r.ic, &r.iz), (&r.ic, &r.dn), (&r.ic, &r.io)] {
        v.push(case("aggregate", "ii", format!("pos {outer} v0\ncount {inner} v0 _ v1\nhead v0 v1"), true));
        v.push(case("aggregate", "ii", format!("pos {outer} v0\ncount {inner} _ v0 v1\nhead v0 v1"), true));
        v.push(case("aggregate", "ii", format!("pos {outer} v0\ncount {inner} _ _ v1\nhead v0 v1"), true));
        for op in ["sum", "min", "max"] {
            // ia holds the extremes: its sums pass 64 bits (the CPU's fold holds 128), which the device refuses.
            let must = !(op == "sum" && std::ptr::eq(inner, &r.ia));
            v.push(case("aggregate", "ii", format!("pos {outer} v0\n{op} {inner} v0 _ 1 v1\nhead v0 v1"), must));
            v.push(case("aggregate", "ii", format!("pos {outer} v0\n{op} {inner} _ v0 0 v1\nhead v0 v1"), must));
            v.push(case("aggregate", "ii", format!("pos {outer} v0\n{op} {inner} _ _ 1 v1\nhead v0 v1"), must));
        }
    }
    for a in [&r.ia, &r.ib] {
        // The value tested, not bound.
        v.push(case("aggregate", "ii", format!("pos {a} v0 v1\nmax {a} v0 _ 1 v1\nhead v0 v1"), true));
        v.push(case("aggregate", "ii", format!("pos {a} v0 v1\nmin {a} v0 _ 1 v1\nhead v0 v1"), true));
        v.push(case("aggregate", "ii", format!("pos {a} v0 v1\ncount {a} v0 _ v2\nlt i:2 v2\nhead v0 v2"), true));
    }
    // A sum past 64 bits (the CPU's fold holds 128): the device refuses it at run time, the rows the same.
    v.push(case("aggregate", "ii", format!("pos {} v0\nsum {} _ _ 0 v1\nhead v0 v1", r.ic, r.ia), false));
    v
}

/// Text tests: every operator, wanted and not, of constants (by automaton) and sets of them, and literal tests of
/// bound patterns.
fn text_tests(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let pats = ["foo", "SRC", ".ts", "αβ", "", "a", "_b", "/", "é", "src/lib*", "^[a-z]+_[a-z]+[0-9]*$", "(?i)todo", "x9$", "o{2}"];
    for op in crate::code_rule_dsl::logic::strings::TESTS {
        for p in pats {
            let rx = op.ends_with("rx");
            if rx != (p.contains('^') || p.contains('$') || p.contains('{') || p.contains("(?")) && !(rx && !p.is_empty()) {
                continue;
            }
            for want in [1, 0] {
                // A Unicode word boundary has no automaton the device holds: refused, the rows the same.
                let must = !op.ends_with("word");
                v.push(case("text test", "si", format!("pos {} v0 v1\ntest {op} v0 {} {want}\nhead v0 v1", r.sn, sym(p)), must));
            }
        }
        v.push(case("text test", "si", format!("pos {} v0 v1\ntestset {op} v0 1 {} {} {}\nhead v0 v1", r.sn, sym("foo"), sym(".ts"), sym("αβ")), !op.ends_with("word")));
    }
    // Sets of literal patterns (empty members among them) over made texts: a cat held, a cat made, a text function's.
    for op in ["equals", "prefix", "suffix"] {
        for want in [1, 0] {
            let set = format!("{} {} {} {}", sym("_"), sym("a"), sym("αβ"), sym("foo.ts"));
            v.push(case("text set made", "si", format!("pos {} v0 v1\ncat v2 v0 {}\ntestset {op} v2 {want} {set}\nhead v0 v1", r.sn, sym("ts")), true));
            v.push(case("text set made", "si", format!("pos {} v0 v1\nfunc str_concat v2 {} v0\ntestset {op} v2 {want} {set}\nhead v2 v1", r.sn, sym("a")), true));
            v.push(case("text set made", "si", format!("pos {} v0 v1\nfunc dir v2 v0\ntestset {op} v2 {want} {set}\nhead v2 v1", r.sn), true));
            v.push(case("text set made", "si", format!("pos {} v0 v1\nfunc sql_upper v2 v0\ntestset {op} v2 {want} {} {}\nhead v2 v1", r.sn, sym("_"), sym("")), true));
            v.push(case("text set made", "si", format!("pos {} v0 v1\nlt v1 i:50\nfunc rr_to_string v2 v1\ntestset {op} v2 {want} {} {} {}\nhead v2 v1", r.sn, sym("1"), sym("4"), sym("10")), true));
        }
    }
    for op in ["equals", "prefix", "suffix"] {
        for want in [1, 0] {
            v.push(case("bound pattern", "si", format!("pos {} v0 v1\npos {} v2 v3\ntest {op} v0 v2 {want}\nhead v0 v3", r.sn, r.sp), true));
            v.push(case("bound pattern", "si", format!("pos {} v0\npos {} v1 v2\ntest {op} v1 v0 {want}\nhead v1 v2", r.sm, r.sn), true));
        }
    }
    v
}

/// The order and length of names, cats held and made (compared, in heads, both), and equalities of names.
fn names(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    for c in ["", "foo", "foo_bar", "src", "αβ", "zz"] {
        for (op, k) in [("lt", "v2 i:0"), ("lt", "i:0 v2"), ("eq", "v2 i:0")] {
            v.push(case("str_cmp", "si", format!("pos {} v0 v1\nfunc str_cmp v2 v0 {}\n{op} {k}\nhead v0 v1", r.sn, sym(c)), true));
        }
        v.push(case("cat", "si", format!("pos {} v0 v1\ncat v2 v0 {}\neq v2 {}\nhead v0 v1", r.sn, sym(c), sym("foo.ts")), true));
        v.push(case("cat", "si", format!("pos {} v0 v1\ncat v2 v0 {}\nhead v2 v1", r.sn, sym(c)), true));
        v.push(case("cat", "si", format!("pos {} v0 v1\nfunc str_concat v2 {} v0\nhead v2 v1", r.sn, sym(c)), true));
        v.push(case("cat", "si", format!("pos {} v0 v1\nfunc str_concat v2 v0 {}\nfunc rr_strlen v3 v2\nlt v3 i:6\nhead v2 v3", r.sn, sym(c)), true));
    }
    v.push(case("str_cmp", "si", format!("pos {} v0\npos {} v1 v2\nfunc str_cmp v3 v1 v0\neq v3 i:0\nhead v1 v2", r.sm, r.sn), true));
    v.push(case("str_cmp", "si", format!("pos {} v0\npos {} v1 v2\nlt v2 i:80\nfunc str_cmp v3 v0 v1\nlt i:0 v3\nhead v0 v2", r.sm, r.sn), true));
    v.push(case("strlen", "si", format!("pos {} v0 v1\nfunc rr_strlen v2 v0\nhead v0 v2", r.sn), true));
    v.push(case("strlen", "si", format!("pos {} v0 v1\nfunc rr_strlen v2 v0\nlt i:9 v2\nhead v0 v1", r.sn), true));
    v.push(case("cat", "si", format!("pos {} v0\npos {} v1 v2\ncat v3 v0 {}\neq v3 v1\nhead v1 v2", r.sm, r.sn, sym("ts")), true));
    v.push(case("cat", "si", format!("pos {} v0\npos {} v1 v2\nlt v2 i:300\ncat v3 v0 {}\nne v3 v1\ntest prefix v1 v0 1\nhead v3 v2", r.sm, r.sn, sym("x")), true));
    v.push(case("cat", "si", format!("pos {} v0\npos {} v1 v2\nlt v2 i:40\nhead cat:v0:v1 v2", r.sm, r.sn), true));
    v.push(case("cat", "si", format!("pos {} v0 v1\ncat v2 v0 {}\nfunc sql_upper v3 v2\nhead v3 v1", r.sn, sym("q")), true));
    v.push(case("name eq", "si", format!("pos {} v0\npos {} v1 v2\neq v0 v1\nhead v1 v2", r.sm, r.sn), true));
    v.push(case("name eq", "si", format!("pos {} v0\npos {} v0 v2\nhead v0 v2", r.sm, r.sn), true));
    v.push(case("name eq", "si", format!("pos {} v0 v1\nne v0 {}\nhead v0 v1", r.sn, sym("foo")), true));
    v
}

/// Text functions: each one of constants (per distinct name), of a row's numbers (per row), chains, read by tests,
/// `str_cmp`, `strlen` and other functions and joined with names; replacements of regexes and literals.
fn text_functions(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let sn = &r.sn;
    let one = ["sql_lower", "sql_upper", "head", "dir"];
    for f in one {
        v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc {f} v2 v0\nhead v2 v1"), true));
        v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc {f} v2 v0\nfunc rr_strlen v3 v2\nhead v2 v3"), true));
        v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc {f} v2 v0\ntest prefix v2 {} 1\nhead v0 v1", sym("s")), true));
        v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc {f} v2 v0\nfunc str_cmp v3 v2 {}\nlt v3 i:0\nhead v0 v1", sym("m")), true));
        v.push(case("text fn", "si", format!("pos {} v0\npos {sn} v1 v2\nfunc {f} v3 v1\neq v3 v0\nhead v1 v2", r.sm), true));
    }
    // `lower` past ASCII is the host's: the stratum's names have some, so it is refused, the rows the same.
    v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc lower v2 v0\nhead v2 v1"), false));
    for (f, i, n) in [("rr_substr", 0, 3), ("rr_substr", 1, 4), ("rr_substr", -2, 5), ("rr_substr", 3, -2), ("rr_substr", 0, 0), ("rr_substr", 50, 3),
                      ("sql_substr", 1, 3), ("sql_substr", 0, 2), ("sql_substr", -3, 2), ("sql_substr", 2, -1),
                      ("slice", 0, 3), ("slice", 1, -1), ("slice", -3, 100), ("slice", 4, 2), ("slice", -100, -2)] {
        v.push(case("text fn", "si", format!("pos {sn} v0 v1\nfunc {f} v2 v0 i:{i} i:{n}\nhead v2 v1"), true));
        // A row's numbers: per row.
        v.push(case("text fn row", "si", format!("pos {sn} v0 v1\nlt v1 i:60\nfunc rr_sub v3 v1 i:30\nfunc {f} v2 v0 i:{i} v3\nhead v2 v1"), true));
    }
    v.push(case("text fn row", "si", format!("pos {sn} v0 v1\nlt v1 i:300\nfunc rr_sub v2 i:150 v1\nfunc rr_to_string v3 v2\nhead v3 v1"), true));
    v.push(case("text fn row", "si", format!("pos {sn} v0 v1\nfunc rr_to_string v2 v1\nfunc str_cmp v3 v2 {}\nlt v3 i:0\nhead v2 v1", sym("2")), true));
    // Chains (per distinct name), and a chain over a cat's text (per row).
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc rr_substr v2 v0 i:0 i:6\nfunc sql_upper v3 v2\nfunc rr_strlen v4 v3\nlt v4 i:5\nhead v3 v1"), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc dir v2 v0\nfunc head v3 v2\nfunc sql_lower v4 v3\nhead v4 v1"), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc replace_all v2 v0 {} {}\nfunc slice v3 v2 i:1 i:-1\nhead v3 v1", sym("[0-9]+"), sym("#")), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc sql_upper v2 v0\nfunc sql_replace v3 v2 {} {}\nfunc sql_lower v4 v3\nhead v4 v1", sym("SRC"), sym("<s>")), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\ncat v2 v0 {}\nfunc sql_upper v3 v2\nfunc head v4 v3\nhead v4 v1", sym("t.x")), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc head v2 v0\nfunc dir v3 v0\neq v2 v3\nhead v0 v1"), true));
    // A chain's intermediate text read by the GPU after its maker dropped it, then a text test's verdicts of the
    // same column (its buffer reused while in flight, once).
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc replace_all v2 v0 {} {}\nfunc sql_upper v3 v2\ntestset equals v0 0 {} {}\nhead v3 v1", sym("\\s+"), sym(""), sym("_"), sym("TODO")), true));
    v.push(case("text fn chain", "si", format!("pos {sn} v0 v1\nfunc rr_substr v2 v0 i:1 i:9\nfunc sql_lower v3 v2\nfunc head v4 v3\ntest prefix v0 {} 0\nhead v4 v1", sym("a")), true));
    for (p, w) in [("_", ""), ("_", "--"), ("", "x"), ("src", "SRC"), ("αβ", "ab"), ("a", "aa"), ("--", "-")] {
        v.push(case("replace", "si", format!("pos {sn} v0 v1\nfunc sql_replace v2 v0 {} {}\nhead v2 v1", sym(p), sym(w)), true));
    }
    for p in ["^\\s+|\\s+$", "\\s+", "_+|\\.ts$", "^[A-Za-z]+|[0-9]", "[αβé]+", "[A-Za-z_./αβ]{3,}", "(ab|a)(c|bcd)", "a+b|ab+c", "^node:", "x9$", "\\*", "%2[Cc]"] {
        for w in ["", "~", "<u>"] {
            v.push(case("regex replace", "si", format!("pos {sn} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym(p), sym(w)), true));
        }
        v.push(case("regex replace", "si", format!("pos {sn} v0 v1\nfunc rx_replace v2 v0 {} {}\nfunc rr_strlen v3 v2\nhead v2 v3", sym(p), sym(".")), true));
    }
    // Refused by design (the rows the same): an empty match, a word boundary, a group named in the replacement,
    // a regex of a row's text.
    for (p, w) in [("a*", "-"), ("\\bfoo", "-"), ("(o)", "$1"), ("(?m)^x", "-")] {
        v.push(case("regex replace", "si", format!("pos {sn} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym(p), sym(w)), false));
    }
    v
}

/// Texts made per row read a SIMD group a row (`ROWTEXT`): text tests by automaton, regex replacements and cats of
/// cats, of text functions' texts and of numbers' texts, long ones past a chunk of 32 bytes among them.
fn row_texts(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let sn = &r.sn;
    // The made texts: a cat held, a cat made (then in the head), a text function's of a row's number, a number's.
    let makers = [
        format!("cat v2 v0 {}", sym("Long_Suffix/with.parts_and_more_than_32_bytes")),
        format!("func str_concat v2 {} v0", sym("pre/")),
        "func rr_substr v2 v0 i:1 v1".to_owned(),
        "func rr_to_string v2 v1".to_owned(),
        format!("func sql_upper v3 v0\ncat v2 v3 {}", sym("x")),
    ];
    for m in &makers {
        for op in crate::code_rule_dsl::logic::strings::TESTS {
            for p in ["foo", "a", "αβ", "1"] {
                let p = if op.ends_with("rx") { format!("{p}[a-z_]*$") } else if op.ends_with("glob") { format!("*{p}*") } else { p.to_owned() };
                v.push(case("row test", "si", format!("pos {sn} v0 v1\n{m}\ntest {op} v2 {} 1\nhead v2 v1", sym(&p)), true));
            }
            v.push(case("row test", "si", format!("pos {sn} v0 v1\n{m}\ntestset {op} v2 0 {} {}\nhead v0 v1", sym("o"), sym("_")), true));
        }
        for (p, w) in [("_+", "~"), ("[0-9]+", ""), ("^[A-Za-z]", "<>"), ("s$", "S"), ("[A-Za-z_./αβ]{3,}", "#"), ("(ab|a)(c|bcd)", "!"), ("\\.", ".."), ("[αβé]", "")] {
            v.push(case("row replace", "si", format!("pos {sn} v0 v1\n{m}\nfunc replace_all v4 v2 {} {}\nhead v4 v1", sym(p), sym(w)), true));
        }
        v.push(case("row replace", "si", format!("pos {sn} v0 v1\n{m}\nfunc replace_all v4 v2 {} {}\nfunc rx_replace v5 v4 {} {}\nfunc rr_strlen v6 v5\nhead v5 v6", sym("o"), sym("0"), sym("0+"), sym("-")), true));
        // Text functions of a made text, per row (a SIMD group a row, 32 bytes at a time).
        let fns = [
            "func sql_lower v4 v2".to_owned(),
            "func sql_upper v4 v2".to_owned(),
            "func head v4 v2".to_owned(),
            "func dir v4 v2".to_owned(),
            "func rr_substr v4 v2 i:1 i:40".to_owned(),
            "func rr_substr v4 v2 i:-3 v1".to_owned(),
            "func slice v4 v2 i:-5 i:100".to_owned(),
            "func slice v4 v2 i:2 i:-1".to_owned(),
            "func sql_substr v4 v2 i:2 i:-1".to_owned(),
            "func sql_substr v4 v2 i:33 i:4".to_owned(),
            format!("func sql_replace v4 v2 {} {}", sym("o"), sym("<o>")),
            format!("func sql_replace v4 v2 {} {}", sym(""), sym("x")),
            format!("func sql_replace v4 v2 {} {}", sym("_with"), sym("")),
            format!("func sql_replace v4 v2 {} {}", sym("a"), sym("aa")),
        ];
        for f in &fns {
            v.push(case("row fn", "si", format!("pos {sn} v0 v1\n{m}\n{f}\nhead v4 v1"), true));
            v.push(case("row fn", "si", format!("pos {sn} v0 v1\n{m}\n{f}\nfunc rr_strlen v5 v4\nfunc sql_upper v6 v4\nhead v6 v5"), true));
        }
        v.push(case("row cat", "si", format!("pos {sn} v0 v1\n{m}\ncat v4 v2 v0\nhead v4 v1"), true));
        v.push(case("row cat", "si", format!("pos {sn} v0 v1\n{m}\nfunc str_concat v4 v0 v2\ncat v5 v4 {}\ntest contains v5 {} 1\nhead v5 v1", sym("z"), sym(".p")), true));
        v.push(case("row cat", "si", format!("pos {sn} v0 v1\n{m}\ncat v4 {} v2\nfunc head v5 v4\neq v5 {}\nhead v4 v1", sym(""), sym("pre/foo")), true));
        v.push(case("row cat", "si", format!("pos {} v9\npos {sn} v0 v1\n{m}\ncat v4 v2 v9\nfunc str_cmp v5 v4 v9\nlt v5 i:1\nhead v4 v1", r.sm), true));
    }
    // Numbers' texts: the extremes, zero and negatives, and lower of them (ASCII: on the device).
    for a in [&r.ia, &r.ib] {
        v.push(case("row to_string", "si", format!("pos {a} v0 v1\nfunc rr_to_string v2 v1\nhead v2 v0"), true));
        v.push(case("row to_string", "si", format!("pos {a} v0 v1\nfunc rr_sub v3 i:0 v0\nfunc rr_to_string v2 v3\nfunc lower v4 v2\nhead v4 v1"), true));
        v.push(case("row to_string", "si", format!("pos {a} v0 v1\nfunc rr_to_string v2 v1\nfunc rr_substr v3 v2 i:1 i:3\nhead v3 v0"), true));
    }
    // lower of a made text past ASCII is the host's: refused, the rows the same.
    v.push(case("row fn", "si", format!("pos {sn} v0 v1\ncat v2 v0 {}\nfunc lower v3 v2\nhead v3 v1", sym("é")), false));
    v
}

/// Unions of rules in one stratum (several seeds into one head), and recursion: closures, joins of the relation with
/// itself, with filters, arithmetic and negation of an input.
fn combinators(r: &Rels) -> Vec<Case> {
    let mut v = Vec::new();
    let union = |rules: Vec<String>, head: &'static str| Case { family: "union", head, seeds: rules, joins: Vec::new(), must: true };
    v.push(union(vec![format!("pos {} v0 v1\nhead v0 v1", r.ia), format!("pos {} v0 v1\nhead v1 v0", r.ib), format!("pos {} v0\nhead v0 v0", r.ic)], "ii"));
    v.push(union(vec![format!("pos {} v0 v1\nlt v1 i:0\nhead v0 v1", r.ia), format!("pos {} v0 v1\nneg {} v0\nhead v0 v1", r.ia, r.ic)], "ii"));
    v.push(union(
        vec![format!("pos {} v0 v1\nfunc head v2 v0\nhead v2 v1", r.sn), format!("pos {} v0 v1\ncat v2 v0 {}\nhead v2 v1", r.sn, sym("u")), format!("pos {} v0\nhead v0 i:0", r.sm)],
        "si",
    ));
    let rec = |seeds: Vec<String>, joins: Vec<String>, must: bool| Case { family: "recursion", head: "ii", seeds, joins, must };
    let e = &r.ie;
    // Transitive closure (a closure's walk), left and right recursive.
    v.push(rec(vec![format!("pos {e} v0 v1\nhead v0 v1")], vec![format!("pos d0 v0 v1\npos {e} v1 v2\nhead v0 v2")], true));
    v.push(rec(vec![format!("pos {e} v0 v1\nhead v0 v1")], vec![format!("pos {e} v0 v1\npos d0 v1 v2\nhead v0 v2")], true));
    // Both atoms recursive (delta and full).
    v.push(rec(vec![format!("pos {e} v0 v1\nhead v0 v1")], vec!["pos d0 v0 v1\npos c0 v1 v2\nne v2 v0\nhead v0 v2".into(), "pos c0 v0 v1\npos d0 v1 v2\nne v2 v0\nhead v0 v2".into()], true));
    // Bounded by arithmetic: distances under a cap.
    v.push(rec(vec![format!("pos {e} v0 v1\nhead v0 i:1")], vec![format!("pos d0 v0 v1\npos {e} v0 v2\nfunc rr_add v3 v1 i:1\nlt v3 i:6\nhead v2 v3")], true));
    // With a filter and the negation of an input.
    v.push(rec(vec![format!("pos {e} v0 v1\nlt v0 i:40\nhead v0 v1")], vec![format!("pos d0 v0 v1\npos {e} v1 v2\nneg {} v2\nhead v0 v2", r.ic)], true));
    // From an empty seed: no rounds.
    v.push(rec(vec![format!("pos {} v0 v1\nhead v0 v1", r.iz)], vec![format!("pos d0 v0 v1\npos {e} v1 v2\nhead v0 v2")], true));
    v
}

/// Random strata of integer steps: an atom, then joins by a bound slot, negations, filters, lets, arithmetic, sets,
/// documents and aggregates in any order, two bound slots in the head.
fn random_integers(r: &Rels, n: usize, seed: u64) -> Vec<Case> {
    let mut g = Rng(seed);
    let two = [&r.ia, &r.ib, &r.ie, &r.dn, &r.io];
    let ops = ["rr_add", "rr_sub", "rr_mul", "rr_div", "mod", "rr_max", "rr_min"];
    (0..n)
        .map(|_| {
            let mut steps = vec![format!("pos {} v0 v1", g.pick(&two))];
            let mut bound = 2usize;
            for _ in 0..1 + g.next(5) {
                let a = g.next(bound as u64);
                let b = g.next(bound as u64);
                let k = *g.pick(&INTS);
                let s = match g.next(10) {
                    0 => {
                        bound += 1;
                        if g.chance(50) { format!("pos {} v{a} v{}", g.pick(&two), bound - 1) } else { format!("pos {} v{} v{a}", g.pick(&two), bound - 1) }
                    }
                    1 => match g.next(3) {
                        0 => format!("neg {} v{a}", r.ic),
                        1 => format!("neg {} v{a} _", g.pick(&two)),
                        _ => format!("neg {} v{a} v{b}", g.pick(&two)),
                    },
                    2 => format!("{} v{a} {}", g.pick(&["lt", "le", "eq", "ne"]), if g.chance(50) { format!("v{b}") } else { format!("i:{k}") }),
                    3 => format!("{} i:{k} v{a}", g.pick(&["lt", "le"])),
                    4 | 5 => {
                        bound += 1;
                        let rhs = if g.chance(60) { format!("v{b}") } else { format!("i:{k}") };
                        format!("func {} v{} v{a} {rhs}", g.pick(&ops), bound - 1)
                    }
                    6 => {
                        bound += 1;
                        if g.chance(50) { format!("eq v{} v{a}", bound - 1) } else { format!("eq v{} i:{k}", bound - 1) }
                    }
                    7 => format!("in v{a} {} i:0 i:3 i:{k}", g.next(2)),
                    8 => match g.next(3) {
                        0 => {
                            bound += 1;
                            format!("docof v{} v{a}", bound - 1)
                        }
                        1 => format!("deq v{a} v{b}"),
                        _ => format!("dne v{a} v{b}"),
                    },
                    _ => {
                        bound += 1;
                        let rel = *g.pick(&[&r.ia, &r.ib, &r.dn]);
                        match g.next(4) {
                            0 => format!("count {rel} v{a} _ v{}", bound - 1),
                            1 => format!("min {rel} _ v{a} 0 v{}", bound - 1),
                            2 => format!("max {rel} v{a} _ 1 v{}", bound - 1),
                            _ => format!("sum {rel} v{a} _ 1 v{}", bound - 1),
                        }
                    }
                };
                steps.push(s);
            }
            let h0 = g.next(bound as u64);
            let h1 = g.next(bound as u64);
            steps.push(format!("head v{h0} v{h1}"));
            // A random shape may hold a form the device has not (a sum past 64 bits): its rows the same.
            case("random integers", "ii", steps.join("\n"), false)
        })
        .collect()
}

/// Random strata of text steps over names: tests (by automaton, sets, of bound patterns), text functions and chains of
/// them, regex and literal replacements, `str_cmp`, `strlen`, cats, joins and equalities of names, a number's text.
fn random_texts(r: &Rels, n: usize, seed: u64) -> Vec<Case> {
    let mut g = Rng(seed);
    let tests = crate::code_rule_dsl::logic::strings::TESTS;
    let pats = ["foo", "src", ".ts", "αβ", "a", "_", "x9", "TODO", "/"];
    let fns = ["sql_lower", "sql_upper", "head", "dir"];
    let rxs = ["\\s+", "_+", "[0-9]", "^[a-z]+", "[αβé]+", "o+$"];
    (0..n)
        .map(|_| {
            let mut steps = vec![format!("pos {} v0 v1", r.sn)];
            // Text slots and number slots bound so far.
            let (mut texts, mut nums, mut next) = (vec![0usize], vec![1usize], 2usize);
            for _ in 0..1 + g.next(4) {
                let t = *g.pick(&texts);
                let p = *g.pick(&pats);
                let s = match g.next(11) {
                    0 => format!("test {} v{t} {} {}", g.pick(&tests), sym(p), g.next(2)),
                    1 => format!("testset {} v{t} {} {} {}", g.pick(&["prefix", "suffix", "contains", "equals"]), g.next(2), sym(p), sym(g.pick::<&str>(&pats))),
                    2 => {
                        texts.push(next);
                        next += 1;
                        format!("func {} v{} v{t}", g.pick(&fns), next - 1)
                    }
                    3 => {
                        texts.push(next);
                        next += 1;
                        let f = g.pick(&["rr_substr", "slice", "sql_substr"]);
                        let b = if g.chance(70) { format!("i:{}", g.next(8) as i64 - 3) } else { format!("v{}", g.pick(&nums)) };
                        format!("func {f} v{} v{t} i:{} {b}", next - 1, g.next(6) as i64 - 2)
                    }
                    4 => {
                        texts.push(next);
                        next += 1;
                        if g.chance(50) {
                            format!("func replace_all v{} v{t} {} {}", next - 1, sym(g.pick::<&str>(&rxs)), sym(g.pick::<&str>(&["", "#", "<>"])))
                        } else {
                            format!("func sql_replace v{} v{t} {} {}", next - 1, sym(p), sym(g.pick::<&str>(&["", "#", "<>"])))
                        }
                    }
                    5 => {
                        nums.push(next);
                        next += 1;
                        format!("func rr_strlen v{} v{t}", next - 1)
                    }
                    6 => {
                        nums.push(next);
                        next += 1;
                        let other = if g.chance(50) { sym(p) } else { format!("v{}", g.pick(&texts)) };
                        format!("func str_cmp v{} v{t} {other}", next - 1)
                    }
                    7 => {
                        texts.push(next);
                        next += 1;
                        if g.chance(50) { format!("cat v{} v{t} {}", next - 1, sym(p)) } else { format!("func str_concat v{} {} v{t}", next - 1, sym(p)) }
                    }
                    8 => format!("{} v{t} {}", g.pick(&["eq", "ne"]), if g.chance(60) { sym(p) } else { format!("v{}", g.pick(&texts)) }),
                    9 => format!("{} v{} i:{}", g.pick(&["lt", "le"]), g.pick(&nums), g.next(300)),
                    _ => {
                        texts.push(next);
                        next += 1;
                        format!("func rr_to_string v{} v{}", next - 1, g.pick(&nums))
                    }
                };
                steps.push(s);
            }
            steps.push(format!("head v{} v{}", g.pick(&texts), g.pick(&nums)));
            case("random texts", "si", steps.join("\n"), true)
        })
        .collect()
}

/// Random strata of text and integer steps mixed: everything `random_texts` makes, integer arithmetic and filters of
/// the numbers, aggregates keyed by a name or a made text (over the names) and by a number, and string joins: atoms
/// over the names keyed by a slot holding a name or a made text (positive and negated), and integer joins.
fn random_mixed(r: &Rels, n: usize, seed: u64) -> Vec<Case> {
    let mut g = Rng(seed);
    let tests = crate::code_rule_dsl::logic::strings::TESTS;
    let pats = ["foo", "src", ".ts", "αβ", "a", "_", "x9", "TODO", "/"];
    let fns = ["sql_lower", "sql_upper", "head", "dir"];
    let ints = [&r.ia, &r.ib, &r.ie];
    let ops = ["rr_add", "rr_sub", "rr_mul", "mod", "rr_max", "rr_min"];
    (0..n)
        .map(|_| {
            // Start from the names, or from names and numbers joined by a number.
            let mut steps = vec![format!("pos {} v0 v1", r.sn)];
            let (mut texts, mut nums, mut next) = (vec![0usize], vec![1usize], 2usize);
            if g.chance(30) {
                steps.push(format!("pos {} v1 v2", g.pick(&ints)));
                nums.push(2);
                next = 3;
            }
            for _ in 0..1 + g.next(5) {
                let t = *g.pick(&texts);
                let k = *g.pick(&nums);
                let p = *g.pick(&pats);
                let mut fresh = |pool: &mut Vec<usize>| {
                    pool.push(next);
                    next += 1;
                    next - 1
                };
                let s = match g.next(14) {
                    0 => format!("test {} v{t} {} {}", g.pick(&tests), sym(p), g.next(2)),
                    1 => {
                        let v = fresh(&mut texts);
                        format!("func {} v{v} v{t}", g.pick(&fns))
                    }
                    2 => {
                        let v = fresh(&mut texts);
                        if g.chance(50) { format!("cat v{v} v{t} {}", sym(p)) } else { format!("func str_concat v{v} {} v{t}", sym(p)) }
                    }
                    3 => {
                        let v = fresh(&mut texts);
                        format!("func rr_to_string v{v} v{k}")
                    }
                    4 => {
                        let v = fresh(&mut texts);
                        format!("func rr_substr v{v} v{t} i:{} v{k}", g.next(4) as i64 - 1)
                    }
                    5 => {
                        let v = fresh(&mut nums);
                        format!("func rr_strlen v{v} v{t}")
                    }
                    6 => {
                        let rhs = if g.chance(60) { format!("v{}", g.pick(&nums)) } else { format!("i:{}", g.pick(&INTS)) };
                        let v = fresh(&mut nums);
                        format!("func {} v{v} v{k} {rhs}", g.pick(&ops))
                    }
                    7 => format!("{} v{k} i:{}", g.pick(&["lt", "le", "ne"]), g.next(120) as i64 - 20),
                    // Aggregates keyed by a text (over the names) or by a number (over the integers).
                    8 => {
                        let v = fresh(&mut nums);
                        match g.next(4) {
                            0 => format!("count {} v{t} _ v{v}", r.sn),
                            1 => format!("min {} v{t} _ 1 v{v}", r.sn),
                            2 => format!("max {} v{t} _ 1 v{v}", r.sn),
                            _ => format!("count {} v{k} _ v{v}", g.pick(&ints)),
                        }
                    }
                    // String joins: an atom over the names keyed by a text slot.
                    9 => {
                        let v = fresh(&mut nums);
                        format!("pos {} v{t} v{v}", r.sn)
                    }
                    10 => format!("pos {} v{t}", r.sm),
                    11 => match g.next(2) {
                        0 => format!("neg {} v{t}", r.sm),
                        _ => format!("neg {} v{t} _", r.sn),
                    },
                    // An integer join from a number.
                    12 => {
                        let v = fresh(&mut nums);
                        format!("pos {} v{k} v{v}", g.pick(&ints))
                    }
                    _ => format!("{} v{t} {}", g.pick(&["eq", "ne"]), if g.chance(50) { sym(p) } else { format!("v{}", g.pick(&texts)) }),
                };
                steps.push(s);
            }
            steps.push(format!("head v{} v{}", g.pick(&texts), g.pick(&nums)));
            // A random shape may hold a form the device has not by design (a made text read as an atom's or an
            // aggregate's key: the arena's texts are no symbols): its rows the same.
            case("random mixed", "si", steps.join("\n"), false)
        })
        .collect()
}

macro_rules! parity_test {
    ($name:ident, $what:literal, $cases:expr) => {
        #[test]
        fn $name() {
            if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
                return;
            }
            let fx = fixture();
            let r = rels();
            let cases: Vec<Case> = $cases(&r);
            let (failures, tally) = check(&fx, &cases);
            report($what, &failures, &tally);
        }
    };
}

parity_test!(parity_relational, "joins, semi and anti joins, filters, lets, sets, documents", relational);
parity_test!(parity_arithmetic, "integer arithmetic", arithmetic);
parity_test!(parity_aggregates, "aggregates", aggregates);
parity_test!(parity_text_tests, "text tests", text_tests);
parity_test!(parity_names, "names: order, length, cats, equalities", names);
parity_test!(parity_text_functions, "text functions and replacements", text_functions);
parity_test!(parity_row_texts, "texts made per row: tests, replacements, cats", row_texts);
parity_test!(parity_combinators, "unions and recursion", combinators);
parity_test!(parity_random_integers, "random integer strata", |r: &Rels| [0xa11ce, 0x1d, 0xfeed].into_iter().flat_map(|s| random_integers(r, 400, s)).collect());
parity_test!(parity_random_texts, "random text strata", |r: &Rels| [0xb0b, 0x7e47, 0xc0de].into_iter().flat_map(|s| random_texts(r, 400, s)).collect());
parity_test!(parity_random_mixed, "random mixed text and integer strata", |r: &Rels| [0x31, 0x5157, 0xab1e].into_iter().flat_map(|s| random_mixed(r, 400, s)).collect());


/// Cats made per row past the arena's first size: the stage runs again from the first made text, at the size wanted,
/// the rows the CPU's.
#[test]
fn parity_row_texts_past_the_arena() {
    if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
        return;
    }
    let fx = fixture();
    let r = rels();
    let c = case("row cat", "si", format!("pos {} v9\npos {} v0 v1\nfunc rr_to_string v2 v1\ncat v3 v2 v0\ncat v4 v3 v9\ntest contains v4 {} 0\nhead v4 v1", r.sm, r.sn, sym("zz")), true);
    let (drows, dcounts, report) = run(&fx, &c, 0, true).unwrap();
    let (crows, ccounts, _) = run(&fx, &c, 0, false).unwrap();
    let report = report.expect("a device report");
    assert!(report.refused.is_none(), "{report:?}");
    assert!(report.arena_reruns >= 1, "the cats fill the arena's first size: {report:?}");
    assert!(crows.len() > 10_000, "{} rows", crows.len());
    assert_eq!(drows, crows, "the device's rows are the CPU's");
    assert_eq!(dcounts, ccounts);
}

