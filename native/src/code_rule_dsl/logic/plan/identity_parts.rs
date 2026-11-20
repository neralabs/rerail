//! The parts of a relation a relation's identity (`plan::identities`, decision 0103) is a digest of, read from a
//! version (0173 step 6): its declaration's and its facts' hashes as the image keeps them, and its native's and
//! lattice's fields, which the image keeps decoded (`About`), written as values. A rule is in an identity by its own
//! hash (`RuleRef::hash`), never by a text.
use super::fields::Fields;
use sha2::Digest as _;

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum Native<'v> {
    DfsFirst { edge: &'v str, target: &'v str, roots: Option<&'v str> },
    MatchAll { input: &'v str, pattern: &'v str, groups: Vec<&'v str>, positions: bool },
    Components { edge: &'v str, own: &'v str },
    Reach { seed: &'v str, edge: &'v str },
    ReachThrough { reach: &'v str, map: &'v str },
    Rank { query: &'v str, counted: &'v str, group: usize, op: &'v str },
    Python { kind: &'v str, input: &'v str, targets: Option<&'v str> },
    PythonCandidates { input: &'v str, targets: &'v str, context: &'v str, member: &'v str },
    TextTest { input: &'v str, column: usize, op: &'v str, patterns: Vec<&'v str> },
}

#[derive(Debug)]
struct LatticeDecl<'v> {
    keys: usize,
    ty: &'v str,
    merge: Option<&'v str>,
    widen: Option<u32>,
}

fn native<'v>(f: &Fields<'v>, rel: &str) -> Option<Native<'v>> {
    use super::super::program_image::NativeSpec as S;
    let (r, t) = (|x| f.rel_name(x), |x| f.text(x));
    f.native(rel).map(|n| match &n.spec {
        S::DfsFirst { edge, target, roots } => Native::DfsFirst { edge: r(*edge), target: r(*target), roots: roots.map(r) },
        S::MatchAll { input, pattern, groups, positions } => Native::MatchAll { input: r(*input), pattern: t(*pattern), groups: groups.iter().map(|&g| t(g)).collect(), positions: *positions },
        S::Components { edge, own } => Native::Components { edge: r(*edge), own: r(*own) },
        S::Reach { seed, edge } => Native::Reach { seed: r(*seed), edge: r(*edge) },
        S::ReachThrough { reach, map } => Native::ReachThrough { reach: r(*reach), map: r(*map) },
        S::Rank { query, counted, group, op } => Native::Rank { query: r(*query), counted: r(*counted), group: *group, op: t(*op) },
        S::Python { kind, input, targets } => Native::Python { kind: t(*kind), input: r(*input), targets: targets.map(r) },
        S::PythonCandidates { input, targets, context, member } => Native::PythonCandidates { input: r(*input), targets: r(*targets), context: t(*context), member: t(*member) },
        S::TextTest { input, column, op, patterns } => Native::TextTest { input: r(*input), column: *column, op: t(*op), patterns: patterns.iter().map(|&p| t(p)).collect() },
    })
}

fn lattice<'v>(f: &Fields<'v>, rel: &str) -> Option<LatticeDecl<'v>> {
    f.lattice_decl(rel).map(|l| LatticeDecl { keys: l.keys, ty: f.text(l.ty), merge: l.merge.map(|m| f.text(m)), widen: l.widen })
}

/// `rel`'s parts into `h`: its declaration's and facts' hashes, then (`shape`, a derived relation's) its native, its
/// lattice and whether it is nullary.
pub fn put(h: &mut sha2::Sha256, f: &Fields<'_>, rel: &str, shape: bool) {
    let v = f.v;
    h.update(v.decl_of(rel).map_or(0, |d| d.hash()).to_le_bytes());
    h.update(v.find(rel).and_then(|a| v.facts(a)).map_or(0, |fs| fs.hash()).to_le_bytes());
    if shape {
        h.update(format!("{:?}", (native(f, rel), lattice(f, rel), f.nullary(rel))).as_bytes());
    }
}
