//! Tier 1's record tables (`RecordTables`): the token loop's words, slots, rules and pairs, laid out as the compiled
//! loop reads them, from a block's records. The CPU's tier 1 (`native_cpu_block`) lowers them; tier 2's own records
//! (`native_cpu_stream::fetch_of`) read them too. Nothing here emits code.

use crate::Error;
/// What a token compares past a word's first eight bytes, by the table's longest word: as
/// little as the words need, as the lean lexer's `word_code` compares the ninth byte of its one
/// nine-byte word alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordRest {
    /// No word past eight bytes: nothing.
    None,
    /// Nine bytes at most: the ninth, in its slot's meta's second byte (`META_TOP`), compared
    /// where the token is nine bytes (lean's `(len != 9) | (ninth == b'n')`).
    Ninth,
    /// Sixteen at most: the second eight, in the slot's third word, straight-line.
    Second,
    /// Longer: a hit on a long word calls the tail compare (`tail_fn`).
    Tails,
}

/// A slot's meta, as the lean lexer's `WORDS` entry `(key, len, code)`: each field a load of
/// its own, none cut from a wider word. Byte 0 the length; byte `META_TOP` the ninth byte
/// (`WordRest::Ninth`) or `LONG_WORD` (`WordRest::Tails`); the code bits, a `u32`, at
/// `META_CODE`.
pub const META_TOP: i32 = 1;
pub const META_CODE: i32 = 4;

/// The record pass's tables (`records::WordTable`), in one buffer the code reads by address:
/// each byte's kind (256 bytes), the slots' keys, the slots' `len | code << 8`, each long word's
/// tails (seven words). Byte offsets of each part.
/// A record table's meta bit (`RecordTables::new`): the slot's word is past eight bytes.
pub const LONG_WORD: u64 = 1;

pub struct RecordTables {
    pub words: Vec<u64>,
    /// The word slots, 16 bytes each: the key (the word's first eight bytes), then its meta
    /// (`META_TOP`, `META_CODE`), each field a load, as the lean lexer's `WORDS` entry. Where a word
    /// is past nine bytes and none past 16 (`WordRest::Second`), 32 bytes each: then its bytes
    /// 8 to 16, and a zero.
    pub slots: usize,
    pub tails: usize,
    /// What a token compares past a word's first eight bytes (`WordRest`), by the longest word.
    pub rest: WordRest,
    /// The first bytes whose tokens take no fused rule (`rule_last`), as nibble tables (a bit
    /// where the lookups' `and` is not zero, `set_nibbles`): the lean lexer's `rare`. A
    /// block's rare starts split its starts into runs (`record_tokens`); none where the loop's
    /// starts are not the lean lexer's.
    pub rare: Option<[[u8; 16]; 2]>,
    /// How each rule's tokens end (`TokenEnd`): a rare start's tail is its rule's class's.
    pub ends: Vec<TokenEnd>,
    /// A rare byte's entry is the same with the pair bit or without (`TOKEN[c | pair << 8]`):
    /// a rare start's is loaded by its byte alone.
    pub rare_unpaired: bool,
    pub table: crate::records::WordTable,
    /// The extracts of the first and last bytes.
    pub starts: u16,
    pub lasts: u16,
    /// Where each pair table's rows (a byte a first byte) and sets (four words each) are
    /// (`Records::nexts`).
    pub pairs: Vec<(usize, usize)>,
    /// Each pair table holds no pair (`pair empty`, 0355): its tokens are one byte, no row read.
    pub pairless: Vec<bool>,
    /// Each pair table's triples (`PairTable::thirds`): a pair and the byte after, three bytes.
    pub thirds: Vec<Vec<[u8; 3]>>,
    /// Where each first byte's entry is, eight bytes each (`TOKEN_*`): its kind, its end rule,
    /// whether a word starts with it, its pair row. A token's one load, as the lean lexer's
    /// `TOKEN`. Where the codes are in the entries (`punct_in_entry`), 512 of them, as the lean
    /// lexer's: by the first byte and (`+ 256`) whether it starts a pair.
    pub token: usize,
    /// Each token test's bit in the entries (`TOKEN_TESTS`), where its first byte says whether
    /// it can hold (`test_filter`): set for the first bytes it can.
    pub filters: Vec<Option<i64>>,
    /// Where the records are (`record_layout`): rows where the token loop writes them all.
    pub layout: crate::records::RecordLayout,
    /// The fused rules the entries' bits name (`TOKEN_FUSED_*`): the pair rule and the run
    /// rule `fused_rules` takes with the main run's word in a register. The loop tests the
    /// bits where its own choice is this one, else it compares the rule.
    pub fused: Option<(usize, usize)>,
    /// Where each first byte's punctuation codes are (`TOKEN_PUNCT`), 8 bytes each: the code
    /// bits of its one-byte token, then of its two-byte one, as the word table gives them.
    pub punct: usize,
    /// Where every punctuation's codes fit the entry (`TOKEN_PUNCT_ONE`), the field's width:
    /// the one-byte token's in the first byte's entry, the two-byte one's in its pair's (`+
    /// 256`). Then its code is the entry's, no load beside it and no choice by the length, as
    /// the lean lexer's `TOKEN` gives a punctuation's kind and length by one load. Where the
    /// pair rule's word is the loop's (`pair_nibbles`), whose bit chooses the entry.
    pub punct_in_entry: Option<u32>,
    /// The loop's starts are the lean lexer's (`lean_starts`), for the fused rules (`fused`):
    /// whether pairs chain. The run rule's fused bit is then on its run bytes alone.
    pub lean: Option<bool>,
}

/// The records' layout the compiler chooses for a program (decision 0231): rows where the
/// token loop writes each token's start, last and code itself and they are the only extracts
/// (`records_made`), so the three columns' room is the rows' and nothing else writes it; else
/// columns, as the extracts write their positions.
pub fn record_layout(native: &crate::block_native::BlockNative, t: &RecordTables) -> crate::records::RecordLayout {
    let made = native.records_made();
    let mine = |e: u16| native.extracts.get(usize::from(e)).is_some_and(|k| made.contains(k));
    let both = native.extracts.len() == 2 && (t.starts, t.lasts) == (0, 1) && mine(t.starts) && mine(t.lasts);
    crate::records::RecordLayout::choose(crate::records::RecordWriter::TokenLoop, both)
}

/// The entries' bits from which the token tests' first-byte filters are (`RecordTables::filters`).
pub const TOKEN_TESTS: i64 = 17;

/// The first bytes at which token test `c` can hold, where they decide it (`native_cpu_bytes::
/// needs`): a run's mask at the token's first byte, which needs that byte alone. As the lean
/// lexer tests a `-` or a `0` only at those bytes. None where it lets every first byte of the
/// test's rule through (a number's digits): the bit would decide nothing.
pub fn test_filter(made: &crate::block_native::BlockNative, c: &crate::records::TokenCheck) -> Option<crate::byte_needs::Needs> {
    if c.count || c.least == 0 {
        return None;
    }
    let r = made.records.as_ref()?;
    crate::byte_needs::needs(made, c.o).filter(|n| n.lo == 0 && n.hi == 0 && (0..=255u8).any(|b| r.ends.get(usize::from(b)) == Some(&c.rule) && !n.has(b)))
}

/// A first byte's entry (`RecordTables::token`): the kind in its low byte, then the end rule
/// (`Records::ends`), then 1 where some word starts with it (else no word is looked up, the
/// lean lexer's `words != 0`), then its row in its pair table (`EndRule::Next`).
pub const TOKEN_RULE: i64 = 8;
pub const TOKEN_WORDY: i64 = 16;
pub const TOKEN_PAIR_ROW: i64 = 24;
/// Past the pair row: 1 where the byte's rule is the fused pair rule, and where it is the fused
/// run rule (`RecordTables::fused`), so the loop takes its common tokens by a bit of the entry
/// it loaded, no compare of the rule, as the lean lexer's `TOKEN` gives a punctuation's
/// length and a word's run is its mask's.
pub const TOKEN_FUSED_PAIR: i64 = 32;
pub const TOKEN_FUSED_RUN: i64 = 33;
/// 1 where the byte's tokens are the fused pair rule's and their code bits are by the first
/// byte and the length alone (`RecordTables::punct`): no word looked up, as the lean lexer's
/// `TOKEN` gives a punctuation's kind.
pub const TOKEN_PUNCT: i64 = 34;

/// A punctuation's code in its entry (`RecordTables::punct_in_entry`), past the kind's bits
/// (none set in a code): from this bit to the entry's last.
pub const TOKEN_PUNCT_ONE: u32 = 36;

impl RecordTables {
    /// Whether `b` is a rare first byte (`rare`'s nibble tables).
    pub fn is_rare(&self, b: u8) -> bool {
        self.rare.is_some_and(|n| n[0][usize::from(b & 15)] & n[1][usize::from(b >> 4)] != 0)
    }

    /// How first byte `b`'s tokens end (`TokenEnd`), by its entry's rule.
    pub fn end_at(&self, b: u8) -> TokenEnd {
        self.ends.get(self.rule_at(b)).copied().unwrap_or(TokenEnd::Any)
    }

    /// First byte `b`'s rule, its entry's (`TOKEN_RULE`).
    pub fn rule_at(&self, b: u8) -> usize {
        (self.words[self.token / 8 + usize::from(b)] >> TOKEN_RULE & 0xff) as usize
    }

    pub fn new(r: &crate::records::Records, made: &crate::block_native::BlockNative) -> Result<Self, Error> {
        // A region read in parts (`Reader::parts`, decision 0323): the token loop's walk
        // (`read_in_parts`) writes each part's kind and word, as `records::pass` gives them. Not
        // yet: more than one such reader, one of more than one region, marks or retags (which a
        // part's code would need), a test of its rule, or words past sixteen bytes.
        let parted: Vec<&crate::records::Reader> = r.readers.iter().filter(|rd| rd.parts_in_tier1()).collect();
        if !parted.is_empty() {
            let tested = r.rules.iter().enumerate().any(|(k, e)| matches!(e, crate::records::EndRule::Read(i) if r.readers[usize::from(*i)].parts_in_tier1()) && r.checks.iter().any(|c| usize::from(c.rule) == k));
            // A quiet region (`Reader::quiet`): no mark or retag holds in it, so its parts' codes
            // are plain whatever the grammar's marks.
            // A part's own kind (`Retag::part`) is the walk's, not a retag it would need.
            let marked = parted.iter().any(|rd| !rd.quiet) && (!r.marks.is_empty() || r.retags.iter().any(|t| !t.part) || r.gaps != 0);
            if parted.iter().any(|rd| rd.arms.len() != 1) || marked || tested {
                return Err(Error::UnsupportedFeature("a region read in parts, on the CPU, with this grammar's marks or tests"));
            }
        }
        let table = crate::records::WordTable::new(&r.words).map_err(|_| Error::InvalidTarget(0))?;
        let mut words: Vec<u64> = r.kinds.chunks(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 kinds"))).collect();
        let slots = 8 * words.len();
        // Each slot's length, then the code bits its word gives (`len | bits << 8`): the word at
        // `KIND_BITS`, and the marks of that word alone at `MARK_SHIFT` (`Records::marks`), so
        // that a token takes them with the slot's one load.
        let longest = table.meta.iter().map(|&m| m & 0xff).max().unwrap_or(0);
        let rest = match longest {
            0..=8 => WordRest::None,
            9 => WordRest::Ninth,
            10..=16 => WordRest::Second,
            _ => WordRest::Tails,
        };
        if !parted.is_empty() && rest == WordRest::Tails {
            return Err(Error::UnsupportedFeature("a region read in parts, on the CPU, with words past sixteen bytes"));
        }
        let metas: Vec<u64> = table.meta.iter().map(|&m| {
            let word = u64::from(m >> 8);
            let alone = r.marks.iter().enumerate().filter(|(_, x)| x.0 == crate::records::WORD_ALONE && u64::from(x.1) == word && word != 0).fold(0u64, |a, (k, _)| a | 1 << k);
            // A word past eight bytes: its top bit (`LONG_WORD`), whose other words a token
            // compares; where the words are nine bytes at most, the ninth byte there instead.
            let len = m & 0xff;
            let top = match rest {
                WordRest::Ninth => {
                    let code = (m >> 8) as usize;
                    if len == 9 { table.tails[code - 1][0] & 0xff } else { 0 }
                }
                _ if len > 8 => LONG_WORD,
                _ => 0,
            };
            let bits = word << crate::records::KIND_BITS | alone << crate::records::MARK_SHIFT;
            assert!(bits >> 32 == 0, "a word's code bits past 32");
            u64::from(len) | top << (8 * META_TOP) | bits << (8 * META_CODE)
        }).collect();
        let second = rest == WordRest::Second;
        for (&k, (&m, &w)) in table.keys.iter().zip(metas.iter().zip(&table.meta)) {
            words.extend([k, m]);
            if second {
                // A slot's second eight bytes: its word's, zeros for a short word or none.
                let code = (w >> 8) as usize;
                words.extend([if code == 0 { 0 } else { table.tails[code - 1][0] }, 0]);
            }
        }
        let tails = 8 * words.len();
        for t in &table.tails {
            words.extend(t);
        }
        let mut pairs = Vec::new();
        for t in &r.nexts {
            let rows = 8 * words.len();
            words.extend(t.row.chunks(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 rows"))));
            let sets = 8 * words.len();
            words.extend(t.sets.iter().flatten());
            pairs.push((rows, sets));
        }
        let pairless: Vec<bool> = r.nexts.iter().map(|t| t.sets.iter().all(|x| *x == [0; 4])).collect();
        let token = 8 * words.len();
        // The tests' filters, a bit each past the word flag, as many as fit below the pair row.
        let mut filters = Vec::new();
        let mut bit = TOKEN_TESTS;
        let mut sets = Vec::new();
        for c in &r.checks {
            match test_filter(made, c).filter(|_| bit < TOKEN_PAIR_ROW) {
                Some(n) => {
                    filters.push(Some(bit));
                    sets.push((bit, n, c.rule));
                    bit += 1;
                }
                None => filters.push(None),
            }
        }
        // The fused rules, as the loop chooses them with the main run's word in a register (as
        // `record_tokens` puts it there).
        let main_run: Vec<u16> = main_run(r).into_iter().collect();
        let fused = if r.skip.is_some() { fused_of(r, &filters, Some(made), &main_run) } else { None };
        // The fused pair rule's tokens' code bits, by first byte and length, where one value
        // each is the word table's (the code's own lookup, `WordTable::slot`): every pair of
        // the byte gives the same; no mark of a word read at the token (`Records::marks`).
        let lookup = |text: &[u8]| -> u64 {
            let len = text.len() as u64;
            let key = text.iter().take(8).enumerate().fold(0u64, |a, (i, &b)| a | u64::from(b) << (8 * i));
            let slot = table.slot(key, len);
            match (table.keys.get(slot), metas.get(slot)) {
                (Some(&k), Some(&m)) if k == key && m & 0xff == len => m >> (8 * META_CODE),
                _ => 0,
            }
        };
        let read_marks = r.marks.iter().any(|&(o, wc)| o != crate::records::WORD_ALONE && wc != 0);
        let mut punct_codes = [(0u64, 0u64, false); 256];
        if fused.is_some() && !read_marks {
            // A byte's pair rule (`EndRule::Next`): its own rule, or the other rule of its
            // reader (`/` alone, of the comment's reader), whose tokens not the reader's are
            // that rule's.
            let pair_of = |x: u8| -> Option<&crate::records::PairTable> {
                let next = |k: usize| match r.rules.get(k) {
                    Some(&crate::records::EndRule::Next(i)) => r.nexts.get(usize::from(i)),
                    _ => None,
                };
                next(usize::from(x)).or_else(|| match r.rules.get(usize::from(x)) {
                    Some(&crate::records::EndRule::Read(i)) => r.readers.get(usize::from(i)).and_then(|rd| next(usize::from(rd.other))),
                    _ => None,
                })
            };
            for (b, e) in punct_codes.iter_mut().enumerate() {
                let Some(pt) = r.ends.get(b).and_then(|&x| pair_of(x)) else { continue };
                let one = lookup(&[b as u8]);
                let twos: Vec<u64> = (0..=255u8).filter(|&c| pt.holds(b as u8, c)).map(|c| lookup(&[b as u8, c])).collect();
                let two = twos.first().copied().unwrap_or(0);
                // A byte that starts a triple (`===` after `==`): its codes not by its length
                // alone, so looked up.
                let triple = pt.thirds.iter().any(|t| t[0] == b as u8);
                if one >> 32 == 0 && two >> 32 == 0 && twos.iter().all(|&x| x == two) && !triple {
                    *e = (one, two, true);
                }
            }
        }
        let kb = crate::records::KIND_BITS;
        let width = |x: u64| 64 - (x >> kb).leading_zeros();
        // The codes in the entries are the fused pair rule's (`by_pair`): its bytes' alone.
        let of_fused: Vec<(u64, u64, bool)> = (0..256).filter(|&b| fused.is_some_and(|(kn, _)| r.ends.get(b).is_some_and(|&x| usize::from(x) == kn))).map(|b| punct_codes[b]).collect();
        let low = of_fused.iter().all(|&(one, two, _)| (one | two) & ((1 << kb) - 1) == 0);
        let w = of_fused.iter().fold(0, |a, &(one, two, _)| a.max(width(one)).max(width(two)));
        // The pair's entry by the pair rule's word (`pair_nibbles`), as the loop has it.
        let paired = fused.is_some_and(|(kn, _)| match r.rules[kn] {
            crate::records::EndRule::Next(i) => r.nexts.get(usize::from(i)).is_some_and(|pt| pair_nibbles(r, pt, kn).is_some()),
            _ => false,
        });
        let punct_in_entry = (low && paired && TOKEN_PUNCT_ONE + w <= 64).then_some(w);
        // The lean lexer's starts (`lean_starts`): the run rule's first bytes that are no run
        // byte left to the general tail.
        let lean = match (fused, r.skip) {
            (Some((kn, kr)), Some(skip)) => lean_starts(r, made, skip, kn, kr),
            _ => None,
        };
        let run_word = fused.and_then(|(_, kr)| match r.rules[kr] {
            crate::records::EndRule::Run(o) => Some(o),
            _ => None,
        });
        // Each rule's tokens end plainly (`plain_end`), with the lean lexer's starts.
        // Else, their last byte no pair's first (`word_end`); else any.
        // A rule's first bytes are its own and those of the readers that go on to it
        // (`Reader::other`): its form holds for each of its tokens.
        let mut firsts: Vec<Vec<u8>> = (0..r.rules.len()).map(|k| (0..=255u8).filter(|&b| r.ends.get(usize::from(b)).is_some_and(|&e| usize::from(e) == k)).collect()).collect();
        for _ in 0..r.rules.len() {
            for k in 0..r.rules.len() {
                if let crate::records::EndRule::Read(i) = r.rules[k] {
                    let o = usize::from(r.readers[usize::from(i)].other);
                    if o < r.rules.len() && o != k {
                        let more: Vec<u8> = firsts[k].iter().copied().filter(|b| !firsts[o].contains(b)).collect();
                        firsts[o].extend(more);
                    }
                }
            }
        }
        let ends_of: Vec<TokenEnd> = (0..r.rules.len()).map(|k| {
            let starts = &firsts[k];
            match lean.and(fused) {
                Some((kn, kr)) if plain_end(r, made, k, starts, kn, kr, 0) => TokenEnd::Plain,
                Some((kn, kr)) if word_end(r, made, k, starts, kn, kr) => TokenEnd::Word,
                _ => TokenEnd::Any,
            }
        }).collect();
        let mut entries = [0u64; 512];
        for (at, e) in entries.iter_mut().enumerate() {
            let (b, two) = (at & 255, at >= 256);
            let rule = r.ends.get(b).copied().unwrap_or(crate::records::NO_RULE);
            let wordy = r.words.iter().any(|w| w.first() == Some(&(b as u8)));
            // The row of the byte's pair rule: its own, or the rule its reader goes on to
            // (`Reader::other`: `/` alone or `/=`, of the comments' reader), as `pair_of` above.
            let pair_rule = match r.rules.get(usize::from(rule)) {
                Some(&crate::records::EndRule::Read(i)) => r.readers.get(usize::from(i)).map_or(usize::from(rule), |rd| usize::from(rd.other)),
                _ => usize::from(rule),
            };
            let row = match r.rules.get(pair_rule) {
                Some(&crate::records::EndRule::Next(i)) => r.nexts[usize::from(i)].row.get(b).copied().unwrap_or(0),
                _ => 0,
            };
            // A bit only on its rule's bytes: a test then runs wherever its bit is set, on a path
            // that more than one rule takes (`rule_last`'s fused rules).
            let tests = sets.iter().filter(|(_, n, k)| n.has(b as u8) && *k == rule).fold(0u32, |a, &(k, _, _)| a | 1 << k);
            let (fp, fr) = fused.map_or((false, false), |(kn, kr)| (usize::from(rule) == kn, usize::from(rule) == kr));
            let fr = fr && (lean.is_none() || run_word.is_some_and(|o| run_fused(made, o, b as u8)));
            *e = u64::from(r.kinds[b]) | u64::from(rule) << TOKEN_RULE | u64::from(wordy) << TOKEN_WORDY | u64::from(tests) | u64::from(row) << TOKEN_PAIR_ROW | u64::from(fp) << TOKEN_FUSED_PAIR | u64::from(fr) << TOKEN_FUSED_RUN | u64::from(punct_codes[b].2) << TOKEN_PUNCT;
            if let (Some(_), true, true) = (punct_in_entry, punct_codes[b].2, fp) {
                *e |= (if two { punct_codes[b].1 } else { punct_codes[b].0 } >> kb) << TOKEN_PUNCT_ONE;
            }
        }
        words.extend(entries);
        // The rare first bytes: those of no fused rule (neither entry bit), as the lean
        // lexer's `rare` (its strings', comments', numbers' first bytes).
        let rare = lean.and(fused).and_then(|_| {
            let mut set = [false; 256];
            for (b, x) in set.iter_mut().enumerate() {
                *x = entries[b] & (1 << TOKEN_FUSED_PAIR | 1 << TOKEN_FUSED_RUN) == 0;
            }
            set_nibbles(&set)
        });
        let punct = 8 * words.len();
        words.extend(punct_codes.iter().map(|&(one, two, _)| one | two << 32));
        let rare_unpaired = rare.is_some_and(|n| (0..256).all(|b| n[0][b & 15] & n[1][b >> 4] == 0 || entries[b] == entries[b | 256]));
        Ok(RecordTables { words, slots, tails, rest, rare, ends: ends_of, rare_unpaired, table, starts: r.starts, lasts: r.lasts, pairs, pairless, thirds: r.nexts.iter().map(|t| t.thirds.clone()).collect(), token, filters, layout: crate::records::RecordLayout::Columns, fused, punct, punct_in_entry, lean })
    }
}

/// `RareClass::rule` of the rare bytes with no rule.
pub const NO_RULE: usize = usize::MAX;

/// A pair table as nibble tables (decision 0177), the lean lexer's `FIRST_LO`, `FIRST_HI`,
/// `SECOND_LO`, `SECOND_HI`: each first byte of rule `rule` with a pair its own bit (eight at
/// most), the first byte's bits the lookups' `and` of its nibbles', the second byte's the bits of
/// the first bytes it ends a pair after. Byte `i` starts a pair where the two `and`ed are not
/// zero; none where the tables do not say exactly `holds` for each first byte of the rule.
pub fn pair_nibbles(r: &crate::records::Records, t: &crate::records::PairTable, rule: usize) -> Option<[[u8; 16]; 4]> {
    let firsts: Vec<u8> = (0..=255u8).filter(|&a| r.ends.get(usize::from(a)).is_some_and(|&e| usize::from(e) == rule) && (0..=255u8).any(|b| t.holds(a, b))).collect();
    // No pair (`pair empty`, 0355): tables of zeros, the pair word zero (`pair_word`'s none).
    if firsts.len() > 8 {
        return None;
    }
    let mut n = [[0u8; 16]; 4];
    for (k, &a) in firsts.iter().enumerate() {
        n[0][usize::from(a & 15)] |= 1 << k;
        n[1][usize::from(a >> 4)] |= 1 << k;
        for b in (0..=255u8).filter(|&b| t.holds(a, b)) {
            n[2][usize::from(b & 15)] |= 1 << k;
            n[3][usize::from(b >> 4)] |= 1 << k;
        }
    }
    let class = |lo: &[u8; 16], hi: &[u8; 16], x: u8| lo[usize::from(x & 15)] & hi[usize::from(x >> 4)];
    let exact = (0..=255u8).filter(|&a| r.ends.get(usize::from(a)).is_some_and(|&e| usize::from(e) == rule)).all(|a| (0..=255u8).all(|b| (class(&n[0], &n[1], a) & class(&n[2], &n[3], b) != 0) == t.holds(a, b)));
    // The bytes past the file are zeros: a pair ending in one would hold there.
    let padded = firsts.iter().all(|&a| !t.holds(a, 0));
    (exact && padded).then_some(n)
}

/// `fused_rules` from the tests' filters and the program whose bytes make the loop's words
/// (none where they are not the bytes').
pub fn fused_of(r: &crate::records::Records, filters: &[Option<i64>], native: Option<&crate::block_native::BlockNative>, runs: &[u16]) -> Option<(usize, usize)> {
    use crate::records::EndRule;
    let first_bytes = |k: usize| (0..256).filter(|&b| r.ends.get(b).is_some_and(|&e| usize::from(e) == k)).count();
    let filtered = |k: usize| r.checks.iter().zip(filters).all(|(x, b)| usize::from(x.rule) != k || b.is_some());
    let kn = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], EndRule::Next(_)) && filtered(k) && first_bytes(k) > 0).max_by_key(|&k| first_bytes(k));
    let kr = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], EndRule::Run(o) if runs.contains(&o)) && filtered(k) && first_bytes(k) > 0).max_by_key(|&k| first_bytes(k));
    let no_extra = |ks: [usize; 2]| match r.extra {
        None => true,
        Some(e) => native.zip(r.marks.get(usize::from(e))).and_then(|(b, &(o, _))| crate::byte_needs::needs(b, o)).is_some_and(|n| {
            n.lo == 0 && n.hi == 0 && (0..=255u8).all(|x| !n.has(x) || ks.iter().all(|&k| r.ends.get(usize::from(x)).is_none_or(|&e| usize::from(e) != k)))
        }),
    };
    kn.zip(kr).filter(|&(a, b)| no_extra([a, b]))
}

/// Whether the loop's starts can be the lean lexer's (decision 0177): every byte not the skip's,
/// but a run's bytes after its first (`word & word << 1`) and a pair's second byte. Then a token
/// of the fused rules (`fused_rules`, the pair rule `kn`, the run rule `kr`) covers no start
/// after its own, and the loop clears its bit alone. It holds where the words are the bytes'
/// alone and: no first byte of `kn`, nor a second of its pairs, is a run byte (a pair ends no
/// run, starts none); a pair's second byte and a run byte are no skip's, and a pair's second
/// no zero (the bytes past the file: the pair word is not cut there, the run's is), so a start
/// put back after another rule's token (`record_tokens`) is one.
/// A first byte of `kr` that is no run byte (`#` of `#include`: the run from the byte after
/// it starts there) is then not fused (`run_fused`): its token takes the rules' general tail.
///
/// Some(whether a pair's second byte can start one, `=>=`), as the lean lexer's
/// `left_to_right`; none where the starts cannot be the lean lexer's.
pub fn lean_starts(r: &crate::records::Records, native: &crate::block_native::BlockNative, skip: u16, kn: usize, kr: usize) -> Option<bool> {
    use crate::records::EndRule;
    let (EndRule::Next(pi), EndRule::Run(o)) = (r.rules[kn], r.rules[kr]) else { return None };
    // A triple (`PairTable::thirds`) ends a byte past its pair's second: the pair word's starts
    // do not say where the token after it starts.
    let pt = r.nexts.get(usize::from(pi)).filter(|t| t.thirds.is_empty())?;
    let alone = |o: u16| crate::byte_needs::needs(native, o).filter(|n| n.lo == 0 && n.hi == 0);
    let (word, space) = (alone(o)?, alone(skip)?);
    // A cut (`run R once D`) ends a token before a byte of `D`: one the starts must hold, so no
    // run byte (which they leave out after a run byte), and its bytes those `D` tests alone.
    for c in r.checks.iter().filter(|c| c.then == crate::records::CheckThen::Cut) {
        let d = alone(c.o)?;
        if (0..=255u8).any(|x| d.has(x) && word.has(x)) {
            return None;
        }
    }
    let of = |x: u8, k: usize| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == k);
    let pairs_apart = (0..=255u8).filter(|&a| of(a, kn)).all(|a| !word.has(a) && (0..=255u8).filter(|&b| pt.holds(a, b)).all(|b| !word.has(b) && !space.has(b) && b != 0));
    // A run byte may be zero (JSON's `wordbyte`, `~(ws | op | quote)`): the bytes past the file
    // are then no run's, as the run's word is cut at the file's end (`Bytewise::word`'s `cut64`,
    // where the output is not false on a zero byte), so no start is put back past it (0355).
    let words_start = (0..=255u8).all(|x| !word.has(x) || !space.has(x));
    let starts_pair = |a: u8| of(a, kn) && (0..=255u8).any(|b| pt.holds(a, b));
    let chained = (0..=255u8).filter(|&a| starts_pair(a)).any(|a| (0..=255u8).any(|b| pt.holds(a, b) && starts_pair(b)));
    (pairs_apart && words_start).then_some(chained)
}

/// Whether first byte `b` of the fused run rule (word `o`) takes the fused rules' path where
/// the starts are the lean lexer's (`lean_starts`): it is a run byte, its token the run from it.
pub fn run_fused(native: &crate::block_native::BlockNative, o: u16, b: u8) -> bool {
    crate::byte_needs::needs(native, o).is_some_and(|n| n.has(b))
}

/// How a rule's tokens end, for the starts after them (`lean_starts` leaves out a run byte
/// after a run byte, and a pair's second): as the lean lexer's arms, each what it read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenEnd {
    /// No byte after it left out (`plain_end`): the starts past it.
    Plain,
    /// Its last byte no pair's first (`word_end`): the byte after it put back where it is a
    /// run byte, as the lean lexer's number arm reads the word after it.
    Word,
    /// Either put back, and a run of pairs across its end read again.
    Any,
}

/// The bytes a token of rule `k` may end with (its first bytes `starts`), by its ends'
/// outputs (`native_cpu_bytes::output_byte_at`); `None` where they do not say. A reader's
/// closers at their `off`, its opener's `M` bytes where none closes (none for a token to the
/// file's end: no byte after it), and the rule it goes on to; a last's output; a run's first
/// byte and the bytes it can end with (`run_last_bytes`); a pair's first byte and its seconds.
pub fn last_bytes(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], depth: u32) -> Option<[u64; 4]> {
    use crate::records::{EndRule, ReadNone};
    let or = |a: [u64; 4], b: [u64; 4]| [a[0] | b[0], a[1] | b[1], a[2] | b[2], a[3] | b[3]];
    let mut first = [0u64; 4];
    for &b in starts {
        first[usize::from(b >> 6)] |= 1 << (b & 63);
    }
    match *r.rules.get(k)? {
        EndRule::Read(i) => {
            let rd = r.readers.get(usize::from(i))?;
            let mut set = [0u64; 4];
            for a in &rd.arms {
                set = or(set, crate::byte_needs::output_byte_at(made, a.closer, i32::from(a.off))?);
                if let ReadNone::Bytes(m) = a.none {
                    set = or(set, crate::byte_needs::output_byte_at(made, a.opener, i32::from(m.checked_sub(1)?))?);
                }
            }
            let other = usize::from(rd.other);
            if other < r.rules.len() && other != k {
                set = or(set, if depth < 4 { last_bytes(r, made, other, starts, depth + 1)? } else { return None });
            }
            Some(set)
        }
        EndRule::Last(out) => crate::byte_needs::output_byte_at(made, out, 0),
        EndRule::Run(o) => Some(or(first, crate::byte_needs::run_last_bytes(made, o)?)),
        EndRule::Next(p) => {
            let t = r.nexts.get(usize::from(p))?;
            let mut set = first;
            for &x in starts {
                for y in 0..=255u8 {
                    if t.holds(x, y) {
                        set[usize::from(y >> 6)] |= 1 << (y & 63);
                    }
                }
            }
            Some(set)
        }
        _ => None,
    }
}

/// Whether no token of rule `k` ends with a pair's first byte (the pair rule `kn`, the run rule
/// `kr`, as `plain_end`): the only start left out after it is a run byte after a run byte, so
/// the byte after it is put back where it is a run byte (`TokenEnd::Word`). After a last byte
/// that is no run byte, a run byte is a start already: its bit again changes nothing.
pub fn word_end(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], kn: usize, kr: usize) -> bool {
    use crate::records::EndRule;
    let (EndRule::Next(pi), EndRule::Run(_)) = (r.rules[kn], r.rules[kr]) else { return false };
    let Some(pt) = r.nexts.get(usize::from(pi)) else { return false };
    let of_kn = |x: u8| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == kn);
    let Some(last) = last_bytes(r, made, k, starts, 0) else { return false };
    (0..=255u8).all(|b| last[usize::from(b >> 6)] >> (b & 63) & 1 == 0 || !(of_kn(b) && (0..=255u8).any(|y| pt.holds(b, y))))
}

/// Whether a token of rule `k` ends where the loop's starts (`lean_starts`, the pair rule `kn`,
/// the run rule `kr`) leave out no byte after it: its last byte no run byte and no pair's
/// first, or the byte after it no run byte and no pair's second. Then no start is put back
/// after it. By the bytes its ends' outputs test (`native_cpu_bytes::output_byte_at`): a reader's
/// closer at its `off` (and the one after), its opener's `M` bytes where none closes; a
/// reader it goes on to (`Reader::other`) the same. False where they do not say.
pub fn plain_end(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], kn: usize, kr: usize, depth: u32) -> bool {
    use crate::records::{EndRule, ReadNone};
    let (EndRule::Next(pi), EndRule::Run(o)) = (r.rules[kn], r.rules[kr]) else { return false };
    let Some(word) = crate::byte_needs::needs(made, o).filter(|n| n.lo == 0 && n.hi == 0) else { return false };
    let Some(pt) = r.nexts.get(usize::from(pi)) else { return false };
    let of_kn = |x: u8| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == kn);
    let firsts: Vec<bool> = (0..=255u8).map(|x| of_kn(x) && (0..=255u8).any(|y| pt.holds(x, y))).collect();
    let seconds: Vec<bool> = (0..=255u8).map(|y| (0..=255u8).any(|x| of_kn(x) && pt.holds(x, y))).collect();
    let none_of = |set: Option<[u64; 4]>, risky: &dyn Fn(u8) -> bool| set.is_some_and(|s| (0..=255u8).all(|b| s[usize::from(b >> 6)] >> (b & 63) & 1 == 0 || !risky(b)));
    let last_risky = |b: u8| word.has(b) || firsts[usize::from(b)];
    let after_risky = |b: u8| word.has(b) || seconds[usize::from(b)];
    let safe = |out: u16, at: i32| none_of(crate::byte_needs::output_byte_at(made, out, at), &last_risky) || none_of(crate::byte_needs::output_byte_at(made, out, at + 1), &after_risky);
    match r.rules.get(k) {
        Some(&EndRule::Read(i)) => {
            let Some(rd) = r.readers.get(usize::from(i)) else { return false };
            let arms = rd.arms.iter().all(|a| {
                let closed = safe(a.closer, i32::from(a.off));
                let open = match a.none {
                    ReadNone::Refuse | ReadNone::End => true,
                    ReadNone::Bytes(m) => m > 0 && safe(a.opener, i32::from(m) - 1),
                };
                closed && open
            });
            let other = usize::from(rd.other);
            arms && (other >= r.rules.len() || other == k || (depth < 4 && plain_end(r, made, other, starts, kn, kr, depth + 1)))
        }
        Some(&EndRule::Last(out)) => safe(out, 0),
        // A pair rule's token ends at its first byte, where no second follows it (any byte
        // the pair row has not), or at the second, whose next byte is any.
        Some(&EndRule::Next(p)) => r.nexts.get(usize::from(p)).is_some_and(|t| starts.iter().all(|&x| {
            let alone = !last_risky(x) || (0..=255u8).all(|y| t.holds(x, y) || !after_risky(y));
            alone && (0..=255u8).all(|y| !t.holds(x, y) || !last_risky(y))
        })),
        _ => false,
    }
}

/// A set of bytes as nibble tables: byte `x` is in it where `lo[x & 15] & hi[x >> 4]` is not
/// zero. Each distinct set of low nibbles a high nibble has is a bit (eight at most); none
/// past eight.
pub fn set_nibbles(set: &[bool; 256]) -> Option<[[u8; 16]; 2]> {
    let mut groups: Vec<u16> = Vec::new();
    let mut n = [[0u8; 16]; 2];
    for h in 0..16 {
        let lows = (0..16).filter(|&l| set[h << 4 | l]).fold(0u16, |a, l| a | 1 << l);
        if lows == 0 {
            continue;
        }
        let g = match groups.iter().position(|&x| x == lows) {
            Some(g) => g,
            None => {
                groups.push(lows);
                groups.len() - 1
            }
        };
        if g >= 8 {
            return None;
        }
        n[1][h] = 1 << g;
    }
    for (g, &lows) in groups.iter().enumerate() {
        for (l, n0) in n[0].iter_mut().enumerate() {
            if lows >> l & 1 == 1 {
                *n0 |= 1 << g;
            }
        }
    }
    Some(n)
}

/// The bytes a token of rule `k` can start with: those whose end rule is `k` (`Records::ends`,
/// the dispatch's, by the first byte alone), and those of a reader whose `other` it is, which
/// goes on to it with the same first byte.
pub fn first_bytes(r: &crate::records::Records, k: usize) -> [u64; 4] {
    let mut set = [0u64; 4];
    for b in 0..256usize {
        let mut at = r.ends.get(b).map(|&e| usize::from(e));
        let mut seen = 0;
        while let Some(j) = at.filter(|&j| j < r.rules.len()) {
            if j == k {
                set[b / 64] |= 1 << (b % 64);
                break;
            }
            seen += 1;
            at = match r.rules[j] {
                crate::records::EndRule::Read(i) if seen <= r.rules.len() => Some(usize::from(r.readers[usize::from(i)].other)),
                _ => None,
            };
        }
    }
    set
}

/// The run rule most first bytes take (a word's), whose word the token loop keeps a block.
pub fn main_run(r: &crate::records::Records) -> Option<u16> {
    let first_bytes = |k: usize| (0..256).filter(|&b| r.ends.get(b).is_some_and(|&e| usize::from(e) == k)).count();
    let main = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], crate::records::EndRule::Run(_))).max_by_key(|&k| first_bytes(k))?;
    match r.rules[main] {
        crate::records::EndRule::Run(o) => Some(o),
        _ => None,
    }
}
