//! Byte classes packed into shared nibble tables, as simdjson packs its character classes.
//!
//! A body of generated code asks of each byte in which of several sets it lies: whitespace,
//! a word's byte, a byte that starts a rare token. simdjson answers all of them with one
//! lookup: two 16-byte tables, one by the byte's low nibble and one by its high nibble, whose
//! `and` gives eight bits. Each bit is a *product*, a set of high nibbles times a set of low
//! nibbles; each class is the union of some bits, and one `vtst` a class reads them.
//!
//! [`pack`] chooses the products for a body's sets so that they fit as few tables as can be,
//! by these means, each of which a hand-written lexer's tables use:
//!
//! - **Shared products.** A product inside two sets serves both (a digit is a word's byte and
//!   a number's first byte).
//! - **Don't-cares.** A byte the body never asks about (`Need::care`) may be in a set or out
//!   of it, whichever lets rows merge into fewer products.
//! - **Ranges.** A run of bytes, wrapping past 0xff, is a subtract and a compare
//!   (`b - lo <= span`): the control bytes and every byte past 0x7f in one test, no product.
//! - **Complements.** A set whose complement takes fewer products is tested by it: the lane's
//!   polarity is free.
//!
//! A set's products all lie in one table, so a set is one lookup; a set of more than eight
//! products takes more than one table, its lookups ORed.

use crate::block::ClassTable;

/// A set of bytes, bit `b` for byte `b`.
pub type ByteSet = [u64; 4];

/// A product of nibble sets: bit `h` of `.0` (high nibbles) times bit `l` of `.1` (low).
pub type Product = (u16, u16);

/// A set a body tests: the bytes in it, and the bytes whose answer matters. Outside `care`,
/// either answer will do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Need {
    pub set: ByteSet,
    pub care: ByteSet,
}

impl Need {
    /// A set every byte's answer of which matters.
    pub fn exact(set: ByteSet) -> Self {
        Need { set, care: [u64::MAX; 4] }
    }
}

/// How a body tests a set: a byte is in it where the range holds or one of the tables' bits
/// does, the answer complemented where `negated`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Test {
    /// `b - lo <= span`, wrapping: `(lo, span)`.
    pub range: Option<(u8, u8)>,
    /// Each lookup's table and the bits of it the set takes.
    pub tables: Vec<(u16, u8)>,
    pub negated: bool,
}

impl Test {
    /// Whether byte `b` passes, by `tables`.
    pub fn holds(&self, tables: &[ClassTable], b: u8) -> bool {
        let in_range = self.range.is_some_and(|(lo, span)| b.wrapping_sub(lo) <= span);
        let in_table = self.tables.iter().any(|&(t, bits)| tables[usize::from(t)].member(b, bits));
        (in_range || in_table) != self.negated
    }
}

/// The tables and each need's test, in the needs' order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packing {
    pub tables: Vec<ClassTable>,
    pub tests: Vec<Test>,
}

impl Packing {
    /// The number of products, bits set over all tables.
    pub fn products(&self) -> usize {
        self.tables.iter().map(|t| (0..8).filter(|k| t.hi.iter().any(|h| h >> k & 1 == 1)).count()).sum()
    }
}

pub fn has(s: &ByteSet, b: u8) -> bool {
    s[usize::from(b >> 6)] >> (b & 63) & 1 == 1
}

fn and(a: &ByteSet, b: &ByteSet) -> ByteSet {
    std::array::from_fn(|k| a[k] & b[k])
}

fn and_not(a: &ByteSet, b: &ByteSet) -> ByteSet {
    std::array::from_fn(|k| a[k] & !b[k])
}

fn or(a: &ByteSet, b: &ByteSet) -> ByteSet {
    std::array::from_fn(|k| a[k] | b[k])
}

fn not(a: &ByteSet) -> ByteSet {
    std::array::from_fn(|k| !a[k])
}

fn empty(a: &ByteSet) -> bool {
    a.iter().all(|&w| w == 0)
}

fn count(a: &ByteSet) -> u32 {
    a.iter().map(|w| w.count_ones()).sum()
}

fn product_bytes(p: Product) -> ByteSet {
    let mut s = [0u64; 4];
    for h in 0..16u8 {
        if p.0 >> h & 1 == 1 {
            for l in 0..16u8 {
                if p.1 >> l & 1 == 1 {
                    let b = h << 4 | l;
                    s[usize::from(b >> 6)] |= 1 << (b & 63);
                }
            }
        }
    }
    s
}

fn range_bytes(lo: u8, span: u8) -> ByteSet {
    let mut s = [0u64; 4];
    for k in 0..=span {
        let b = lo.wrapping_add(k);
        s[usize::from(b >> 6)] |= 1 << (b & 63);
    }
    s
}

/// Each row's (high nibble's) set of low nibbles.
fn rows(s: &ByteSet) -> [u16; 16] {
    std::array::from_fn(|h| (0..16u8).filter(|&l| has(s, (h as u8) << 4 | l)).fold(0u16, |a, l| a | 1 << l))
}

/// The same set by columns: each low nibble's set of high nibbles.
fn columns(s: &ByteSet) -> [u16; 16] {
    std::array::from_fn(|l| (0..16u8).filter(|&h| has(s, h << 4 | l as u8)).fold(0u16, |a, h| a | 1 << h))
}

/// Products covering `req` inside `allow`, rows grouped: a row joins a group whose bytes it
/// may hold and whose bytes it allows, so don't-cares merge rows. `by_columns`: the same by
/// low nibble.
fn grouped(req: &ByteSet, allow: &ByteSet, by_columns: bool) -> Vec<Product> {
    let (r, a) = if by_columns { (columns(req), columns(allow)) } else { (rows(req), rows(allow)) };
    // (lines, required, allowed)
    let mut groups: Vec<(u16, u16, u16)> = Vec::new();
    let mut order: Vec<usize> = (0..16).filter(|&k| r[k] != 0).collect();
    order.sort_by_key(|&k| std::cmp::Reverse(r[k].count_ones()));
    for k in order {
        match groups.iter_mut().find(|g| r[k] & !g.2 == 0 && g.1 & !a[k] == 0) {
            Some(g) => {
                g.0 |= 1 << k;
                g.1 |= r[k];
                g.2 &= a[k];
            }
            None => groups.push((1 << k, r[k], a[k])),
        }
    }
    groups.into_iter().map(|(lines, req, _)| if by_columns { (req, lines) } else { (lines, req) }).collect()
}

/// The runs a range may take for `req` inside `allow`: each longest run of `allow` (wrapping)
/// holding some of `req`, as `(lo, span)`. A shorter run is never better: the bytes it leaves
/// out are allowed, so the longest run covers them for free. Which run is best is the cover's
/// question, not the run's size: the run that leaves the fewest products (a-z, not A-Z, where
/// `_` and P-Z then share a row).
fn ranges(req: &ByteSet, allow: &ByteSet) -> Vec<(u8, u8)> {
    if empty(req) {
        return Vec::new();
    }
    if count(allow) == 256 {
        return vec![(0, 255)];
    }
    // Runs start just after a byte not allowed.
    (0..=255u8)
        .filter(|&start| has(allow, start) && !has(allow, start.wrapping_sub(1)))
        .map(|start| {
            let mut span = 0u8;
            while span < 255 && has(allow, start.wrapping_add(span + 1)) {
                span += 1;
            }
            (start, span)
        })
        .filter(|&(lo, span)| !empty(&and(req, &range_bytes(lo, span))))
        .collect()
}

/// A need's form: its polarity, its range, and its products (indexes into the pool).
#[derive(Clone, Debug)]
struct Form {
    negated: bool,
    range: Option<(u8, u8)>,
    products: Vec<usize>,
}

/// Covers `req` inside `allow` with products of the pool first (each used where it covers
/// a byte still wanted), then new ones; the fewer new of the two groupings. Gives the pool's
/// products used and the new ones.
fn cover(req: &ByteSet, allow: &ByteSet, pool: &[Product]) -> (Vec<usize>, Vec<Product>) {
    let mut left = *req;
    let mut used = Vec::new();
    loop {
        let best = pool.iter().enumerate().filter(|(k, p)| !used.contains(k) && empty(&and_not(&product_bytes(**p), allow))).map(|(k, p)| (count(&and(&left, &product_bytes(*p))), k)).filter(|&(n, _)| n > 0).max();
        match best {
            Some((_, k)) => {
                used.push(k);
                left = and_not(&left, &product_bytes(pool[k]));
            }
            None => break,
        }
    }
    let (rows, cols) = (grouped(&left, allow, false), grouped(&left, allow, true));
    let fresh = if cols.len() < rows.len() { cols } else { rows };
    // A pool product the new ones make needless is dropped (`used` kept only where it still
    // covers a byte no new product does).
    let new_bytes = fresh.iter().fold([0u64; 4], |a, &p| or(&a, &product_bytes(p)));
    let mut kept: Vec<usize> = Vec::new();
    for &k in &used {
        let others = kept.iter().fold(new_bytes, |a, &j| or(&a, &product_bytes(pool[j])));
        if !empty(&and_not(&and(req, &product_bytes(pool[k])), &others)) {
            kept.push(k);
        }
    }
    (kept, fresh)
}

/// How many of `later`'s needs product `p` could serve, in either polarity: inside the bytes
/// a need allows and covering one it requires.
fn serves(p: Product, later: &[&Need]) -> usize {
    let bytes = product_bytes(p);
    later
        .iter()
        .filter(|n| {
            let dont = not(&n.care);
            [and(&n.set, &n.care), and_not(&n.care, &n.set)].iter().any(|req| empty(&and_not(&bytes, &or(req, &dont))) && !empty(&and(&bytes, req)))
        })
        .count()
}

/// The forms of the needs in `order`, each the one adding the fewest products to the pool;
/// of forms adding as few, the one whose new products the needs after it can share most (a
/// word's digits as a product, not a range, where a number's first byte is a digit too).
fn plan(needs: &[Need], order: &[usize]) -> (Vec<Product>, Vec<Form>) {
    let mut pool: Vec<Product> = Vec::new();
    let mut forms: Vec<Option<Form>> = vec![None; needs.len()];
    for (at, &i) in order.iter().enumerate() {
        let later: Vec<&Need> = order[at + 1..].iter().map(|&j| &needs[j]).collect();
        let n = &needs[i];
        let dont = not(&n.care);
        type Key = (usize, usize, std::cmp::Reverse<usize>, usize, bool);
        let mut best: Option<(Key, Form, Vec<Product>)> = None;
        for negated in [false, true] {
            let req = if negated { and_not(&n.care, &n.set) } else { and(&n.set, &n.care) };
            let allow = or(&req, &dont);
            for range in std::iter::once(None).chain(ranges(&req, &allow).into_iter().map(Some)) {
                let ranged = range.is_some();
                let left = match range {
                    Some((lo, span)) => and_not(&req, &range_bytes(lo, span)),
                    None => req,
                };
                let (used, fresh) = cover(&left, &allow, &pool);
                let shared = fresh.iter().map(|&p| serves(p, &later)).sum();
                let key = (fresh.len(), usize::from(ranged), std::cmp::Reverse(shared), used.len() + fresh.len(), negated);
                if best.as_ref().is_none_or(|b| key < b.0) {
                    let mut products = used;
                    products.extend((0..fresh.len()).map(|k| pool.len() + k));
                    best = Some((key, Form { negated, range, products }, fresh));
                }
            }
        }
        let (_, form, fresh) = best.expect("a form");
        pool.extend(fresh);
        forms[i] = Some(form);
    }
    (pool, forms.into_iter().map(|f| f.expect("each need planned")).collect())
}

/// The needs' tests and their tables, as few lookups as the means above give.
pub fn pack(needs: &[Need]) -> Packing {
    let n = needs.len();
    let size = |i: usize| count(&and(&needs[i].set, &needs[i].care));
    let mut by_size: Vec<usize> = (0..n).collect();
    by_size.sort_by_key(|&i| std::cmp::Reverse(size(i)));
    let orders = [by_size.clone(), (0..n).collect(), (0..n).rev().collect(), by_size.into_iter().rev().collect()];
    let mut best: Option<((usize, usize, usize), Packing)> = None;
    for order in orders {
        let (pool, forms) = plan(needs, &order);
        let packing = place(&pool, forms);
        let key = (packing.tables.len(), packing.products(), packing.tests.iter().filter(|t| t.range.is_some()).count());
        if best.as_ref().is_none_or(|b| key < b.0) {
            best = Some((key, packing));
        }
    }
    let packing = best.map(|b| b.1).unwrap_or_default();
    debug_assert!(needs.iter().zip(&packing.tests).all(|(nd, t)| (0..=255u8).all(|b| !has(&nd.care, b) || t.holds(&packing.tables, b) == has(&nd.set, b))));
    packing
}

/// The forms' products placed in tables of eight, each need's in one table where they fit
/// (more than eight: tables of their own); a lone table that a range alone can replace, gone.
fn place(pool: &[Product], forms: Vec<Form>) -> Packing {
    let mut tables: Vec<Vec<usize>> = Vec::new();
    let mut at: Vec<Vec<usize>> = vec![Vec::new(); forms.len()];
    let mut order: Vec<usize> = (0..forms.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(forms[i].products.len()));
    for i in order {
        let ps = &forms[i].products;
        if ps.is_empty() {
            continue;
        }
        if ps.len() > 8 {
            for chunk in ps.chunks(8) {
                tables.push(chunk.to_vec());
                at[i].push(tables.len() - 1);
            }
            continue;
        }
        let fit = tables.iter().enumerate().map(|(t, held)| (ps.iter().filter(|p| !held.contains(p)).count(), t)).filter(|&(added, t)| tables[t].len() + added <= 8).min();
        let t = match fit {
            Some((_, t)) => t,
            None => {
                tables.push(Vec::new());
                tables.len() - 1
            }
        };
        for &p in ps {
            if !tables[t].contains(&p) {
                tables[t].push(p);
            }
        }
        at[i].push(t);
    }
    let mut out = Packing::default();
    for held in &tables {
        let mut t = ClassTable::default();
        for (bit, &p) in held.iter().enumerate() {
            let (hi, lo) = pool[p];
            for k in 0..16 {
                if hi >> k & 1 == 1 {
                    t.hi[k] |= 1 << bit;
                }
                if lo >> k & 1 == 1 {
                    t.lo[k] |= 1 << bit;
                }
            }
        }
        out.tables.push(t);
    }
    for (i, form) in forms.iter().enumerate() {
        let tables_of: Vec<(u16, u8)> = at[i].iter().map(|&t| {
            let bits = form.products.iter().filter_map(|p| tables[t].iter().position(|q| q == p)).fold(0u8, |a, bit| a | 1 << bit);
            (t as u16, bits)
        }).filter(|&(_, bits)| bits != 0).collect();
        out.tests.push(Test { range: form.range, tables: tables_of, negated: form.negated });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(bytes: impl IntoIterator<Item = u8>) -> ByteSet {
        let mut s = [0u64; 4];
        for b in bytes {
            s[usize::from(b >> 6)] |= 1 << (b & 63);
        }
        s
    }

    /// Each test answers its need on every byte it cares about.
    fn exact(needs: &[Need], p: &Packing) {
        for (k, (n, t)) in needs.iter().zip(&p.tests).enumerate() {
            for b in 0..=255u8 {
                if has(&n.care, b) {
                    assert_eq!(t.holds(&p.tables, b), has(&n.set, b), "need {k} at byte {b:#04x}");
                }
            }
        }
    }

    /// A hand-written lexer's classes (whitespace, a word's bytes, the bytes that start a rare
    /// token: quotes, `#`, `/`, a backtick, digits, a lead byte past 0xbf) are one lookup:
    /// eight products, the digits shared by two sets, the lead bytes a compare.
    #[test]
    fn a_lexers_classes_are_one_lookup() {
        let space = set([b' ', b'\t', b'\n', 0x0c, b'\r']);
        let word = set((b'0'..=b'9').chain(b'a'..=b'z').chain(b'A'..=b'Z').chain([b'_']));
        let rare = set([b'"', b'#', b'/', b'`'].into_iter().chain(b'0'..=b'9').chain(0xc0..=0xff));
        let needs = [Need::exact(space), Need::exact(word), Need::exact(rare)];
        let p = pack(&needs);
        exact(&needs, &p);
        assert_eq!(p.tables.len(), 1, "one table: {p:?}");
        assert!(p.products() <= 8);
        assert!(p.tests[2].range.is_some(), "the lead bytes by a compare");
    }

    /// A product inside two sets is one bit of both.
    #[test]
    fn a_product_in_two_sets_is_shared() {
        let digits = set(b'0'..=b'9');
        let hex = set((b'0'..=b'9').chain(b'a'..=b'f'));
        let ident = set((b'0'..=b'9').chain(b'a'..=b'z'));
        let needs = [Need::exact(hex), Need::exact(ident), Need::exact(set([b'x', b'X'])), Need::exact(or(&digits, &set([b'.'])))];
        let p = pack(&needs);
        exact(&needs, &p);
        // Digits; a-f; g-o and p-z rows; x and X; `.`: each set's own products and the digits once.
        assert!(p.products() <= 7, "{} products: {p:?}", p.products());
        assert_eq!(p.tables.len(), 1);
    }

    /// A lexer's three sets, as its block's window asks them: whitespace, a word's byte, and a
    /// rare token's first byte (whitespace a don't-care: no token starts there). Eight
    /// products, one table, as a hand-written lexer's: the word's a-z a range (`_` and P-Z then
    /// one row), its digits a product the rare set shares, the rare set's quotes and `?` one.
    #[test]
    fn a_lexers_sets_are_one_table() {
        let space = set([b'\t', b'\n', 0x0c, b'\r', b' ']);
        let word = set((b'0'..=b'9').chain(b'A'..=b'Z').chain(b'a'..=b'z').chain([b'_']));
        let rare = set((0x00..=0x20).chain(0x7f..=0xff).chain(b'0'..=b'9').chain(*b"\"#$'/?\\`"));
        let needs = [Need::exact(space), Need::exact(word), Need { set: rare, care: not(&space) }];
        let p = pack(&needs);
        exact(&needs, &p);
        assert_eq!(p.tables.len(), 1, "{p:?}");
        assert!(p.products() <= 8, "{} products: {p:?}", p.products());
    }

    /// A run of bytes wrapping past 0xff (the control bytes, DEL, everything past 0x7f) is
    /// one subtract and compare, no table.
    #[test]
    fn a_wrapping_run_is_a_compare() {
        let odd = set((0x00..=0x1f).chain(0x7f..=0xff));
        let needs = [Need::exact(odd)];
        let p = pack(&needs);
        exact(&needs, &p);
        assert!(p.tables.is_empty(), "{p:?}");
        assert_eq!(p.tests[0].range, Some((0x7f, 0xa0)));
    }

    /// Bytes the body never asks about merge rows: `A`, `B` and `a` with `b` a don't-care is
    /// one product, rows 4 and 6 by low nibbles 1 and 2.
    #[test]
    fn dont_cares_merge_rows() {
        let n = Need { set: set([b'A', b'B', b'a']), care: not(&set([b'b'])) };
        let exact_too = Need::exact(set([b'A', b'B', b'a']));
        let loose = pack(&[n]);
        exact(&[n], &loose);
        let tight = pack(&[exact_too]);
        exact(&[exact_too], &tight);
        assert_eq!(loose.products(), 1, "{loose:?}");
        assert!(tight.products() >= 2 || tight.tests[0].range.is_some());
    }

    /// A set whose complement is fewer products is tested by its complement.
    #[test]
    fn a_cheaper_complement_is_taken() {
        // Every byte but a scattered few, in different rows and columns.
        let few = set([0x13, 0x25, 0x37, 0x49, 0x5b, 0x6d, 0x81, 0x9e]);
        let n = Need::exact(not(&few));
        let p = pack(&[n]);
        exact(&[n], &p);
        assert!(p.tests[0].negated, "{p:?}");
    }

    /// More than eight products take a second table; each set's are in one of them.
    #[test]
    fn past_eight_products_a_second_table() {
        // Nine sets of two bytes, a row each, low nibbles 0 and 2: no run, a product each.
        let needs: Vec<Need> = (0..9u8).map(|h| Need::exact(set([(h + 2) << 4, (h + 2) << 4 | 2]))).collect();
        let p = pack(&needs);
        exact(&needs, &p);
        assert_eq!(p.tables.len(), 2, "{p:?}");
        assert!(p.tests.iter().all(|t| t.tables.len() == 1), "each set one lookup");
    }

    /// Random sets and don't-cares: every test answers its need.
    #[test]
    fn random_needs_are_answered() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..200 {
            let k = 1 + (next() % 5) as usize;
            let needs: Vec<Need> = (0..k).map(|_| {
                // Sparse or dense, by rows.
                let mut s = [0u64; 4];
                let dense = next() % 2 == 0;
                for w in &mut s {
                    *w = if dense { next() | next() } else { next() & next() & next() };
                }
                let care = if next() % 2 == 0 { [u64::MAX; 4] } else { std::array::from_fn(|_| next() | next()) };
                Need { set: s, care }
            }).collect();
            let p = pack(&needs);
            exact(&needs, &p);
            for t in &p.tests {
                assert!(t.tables.iter().all(|&(tb, _)| usize::from(tb) < p.tables.len()));
            }
        }
    }
}
