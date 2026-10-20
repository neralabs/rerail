//! The statement texts of a plan, stored once (`exec::Plan`). A program's texts are many and long
//! (the data-layer policies' are half a million distinct texts, 180 MB), but they are made of few
//! distinct phrases, and a stage's or clause's texts differ from those of another of its shape only
//! in a few numbers of its own: its namespace and run id, and the terminals it reads. So a text is
//! kept as a *skeleton*: its phrases, each stored once, with a *hole* where such a number was. A
//! stage or clause holds the numbers (`exec::Instance`), and a session writes the text a step runs
//! into a buffer (`Skeletons::write`).
//!
//! Writing a text back is exact by construction: a hole stands for a number written in decimal
//! without a leading zero, which is how it is written back, and every other byte of the text is in
//! its phrases, in order.

use foldhash::fast::FixedState;
use hashbrown::HashTable;
use std::hash::BuildHasher;
use std::ops::Range;

/// What a hole's number is: a stage's or clause's own (its namespace's number, `emit::NAMESPACE`
/// and `emit::EACH`, or its run id), or a terminal's id (`term = …`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hole {
    Own = 0,
    Term = 1,
}

/// A hole's number and its kind, `kind << 30 | number`: two holes of a stage or clause with equal
/// params hold one number, so stages or clauses of one shape agree on which of their holes do.
pub(super) type Param = u32;

const NUMBER_BITS: u32 = 30;

/// The number a param holds.
pub(super) fn number(param: Param) -> u32 {
    param & ((1 << NUMBER_BITS) - 1)
}

fn param(hole: Hole, number: u32) -> Param {
    (hole as u32) << NUMBER_BITS | number
}

/// The bytes an identifier or a number is made of.
fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The holes of `text`, in order: each number's byte range, kind and value. A number is a hole
/// where it is a stage's or clause's own (after the first letter of a `c`, `e` or `s` namespace
/// that starts a word, `c<n>_…`; after `run = ` or `run = -`, a whole word) or a terminal's (after
/// `term = `, a whole word), and where it writes back as it is: at most nine digits, no leading
/// zero. Every such number ends just before a `_` or starts just after an `=`, so only those are
/// looked at.
fn holes(text: &str, mut hole: impl FnMut(Range<usize>, Param)) {
    let bytes = text.as_bytes();
    let mut found = |range: Range<usize>, kind: Hole| {
        let digits = &bytes[range.clone()];
        if !digits.is_empty() && digits.len() <= 9 && (digits.len() == 1 || digits[0] != b'0') {
            let number = digits
                .iter()
                .fold(0, |number, digit| number * 10 + u32::from(digit - b'0'));
            hole(range, param(kind, number));
        }
    };
    for at in memchr::memchr2_iter(b'_', b'=', bytes) {
        if bytes[at] == b'_' {
            let count = bytes[..at]
                .iter()
                .rev()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let Some(letter) = (at - count).checked_sub(1) else {
                continue;
            };
            if count > 0
                && matches!(bytes[letter], b'c' | b'e' | b's')
                && (letter == 0 || !word(bytes[letter - 1]))
            {
                found(at - count..at, Hole::Own);
            }
            continue;
        }
        let before = &bytes[..at];
        let kind = if before.ends_with(b"run ") {
            Hole::Own
        } else if before.ends_with(b"term ") {
            Hole::Term
        } else {
            continue;
        };
        let mut start = at + 1;
        if bytes.get(start) != Some(&b' ') {
            continue;
        }
        start += 1;
        if kind == Hole::Own && bytes.get(start) == Some(&b'-') {
            start += 1;
        }
        let end = start
            + bytes[start..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
        if !bytes.get(end).is_some_and(|byte| word(*byte)) {
            found(start..end, kind);
        }
    }
}

/// Where a phrase ends in `text`, from `start`: before the keyword after the first space that
/// precedes one, or at its end. The keywords split a statement where its clauses and conditions
/// start, which is where texts of different statements still share their phrases.
fn phrase_end(text: &[u8], start: usize) -> usize {
    const KEYWORDS: [&[u8]; 12] = [
        b"FROM ", b"JOIN ", b"WHERE ", b"AND ", b"OR ", b"UNION ", b"SELECT ", b"GROUP ",
        b"ORDER ", b"ON ", b"LEFT ", b"WITH ",
    ];
    for space in memchr::memchr_iter(b' ', &text[start..]) {
        let after = start + space + 1;
        if matches!(
            text.get(after),
            Some(b'F' | b'J' | b'W' | b'A' | b'O' | b'U' | b'S' | b'G' | b'L')
        ) && KEYWORDS
            .iter()
            .any(|keyword| text[after..].starts_with(keyword))
        {
            return after;
        }
    }
    text.len()
}

/// A plan's distinct phrases and skeletons. A skeleton's code is a sequence of varints: `0` a
/// hole, `n` the phrase `n - 1`.
#[derive(Default)]
pub(super) struct Skeletons {
    phrases: String,
    phrase_ends: Vec<u32>,
    codes: Vec<u8>,
    code_ends: Vec<u32>,
}

impl Skeletons {
    fn phrase(&self, id: usize) -> &str {
        let start = id
            .checked_sub(1)
            .map_or(0, |before| self.phrase_ends[before]);
        &self.phrases[start as usize..self.phrase_ends[id] as usize]
    }

    fn code(&self, id: usize) -> &[u8] {
        let start = id.checked_sub(1).map_or(0, |before| self.code_ends[before]);
        &self.codes[start as usize..self.code_ends[id] as usize]
    }

    /// The items of skeleton `id`: each phrase, `None` for a hole.
    fn items(&self, id: u32) -> impl Iterator<Item = Option<&str>> {
        let mut code = self.code(id as usize);
        std::iter::from_fn(move || {
            let mut item = 0usize;
            let mut read = 0;
            loop {
                let byte = *code.get(read)?;
                item |= usize::from(byte & 0x7f) << (7 * read);
                read += 1;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            code = &code[read..];
            Some(item.checked_sub(1).map(|phrase| self.phrase(phrase)))
        })
    }

    /// Appends the text of skeleton `id` to `out`, its holes holding `numbers`, in order.
    pub(super) fn write(&self, id: u32, mut numbers: impl FnMut() -> u32, out: &mut String) {
        let mut digits = [0; 10];
        for item in self.items(id) {
            match item {
                Some(phrase) => out.push_str(phrase),
                None => out.extend(
                    decimal(numbers(), &mut digits)
                        .iter()
                        .map(|&digit| char::from(digit)),
                ),
            }
        }
    }

    /// The distinct phrases' and skeletons' bytes, as stored.
    pub(super) fn bytes(&self) -> usize {
        self.phrases.len()
            + 4 * self.phrase_ends.len()
            + self.codes.len()
            + 4 * self.code_ends.len()
    }

    /// How many distinct skeletons there are.
    pub(super) fn len(&self) -> usize {
        self.code_ends.len()
    }
}

/// `number` written in decimal, without a leading zero, in `digits`.
fn decimal(mut number: u32, digits: &mut [u8; 10]) -> &[u8] {
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            return &digits[start..];
        }
    }
}

fn hash(bytes: &[u8]) -> u64 {
    FixedState::default().hash_one(bytes)
}

fn varint(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// The texts of a plan being made: its skeletons, and each distinct text as a skeleton and the
/// params of its holes, by its id. The indexes find a phrase, a skeleton or a text already stored;
/// the plan keeps only the skeletons (`finish`).
#[derive(Default)]
pub(super) struct Texts {
    skeletons: Skeletons,
    phrase_index: HashTable<u32>,
    skeleton_index: HashTable<u32>,
    text_index: HashTable<u32>,
    /// Each text's skeleton, and the end of its params in `params`.
    texts: Vec<(u32, u32)>,
    params: Vec<Param>,
    /// The texts' bytes, each once.
    bytes: usize,
}

impl Texts {
    /// The id of `text`: the same for equal texts.
    pub(super) fn intern(&mut self, text: &str) -> u32 {
        let hashed = hash(text.as_bytes());
        let (skeletons, texts, params) = (&self.skeletons, &self.texts, &self.params);
        if let Some(&id) = self.text_index.find(hashed, |&id| {
            Self::equals(skeletons, texts, params, id, text)
        }) {
            return id;
        }
        let mut code = Vec::new();
        let mut copied = 0;
        holes(text, |range, param| {
            self.phrases(&text[copied..range.start], &mut code);
            code.push(0);
            self.params.push(param);
            copied = range.end;
        });
        self.phrases(&text[copied..], &mut code);
        let skeleton = self.skeleton(&code);
        let id = self.texts.len() as u32;
        self.texts.push((skeleton, self.params.len() as u32));
        self.bytes += text.len();
        let (skeletons, texts, params) = (&self.skeletons, &self.texts, &self.params);
        self.text_index.insert_unique(hashed, id, |&id| {
            let mut text = String::new();
            let (skeleton, held) = Self::text_of(texts, params, id);
            let mut held = held.iter();
            skeletons.write(
                skeleton,
                || number(*held.next().expect("a param")),
                &mut text,
            );
            hash(text.as_bytes())
        });
        id
    }

    fn text_of<'t>(texts: &[(u32, u32)], params: &'t [Param], id: u32) -> (u32, &'t [Param]) {
        let start = (id as usize)
            .checked_sub(1)
            .map_or(0, |before| texts[before].1);
        let (skeleton, end) = texts[id as usize];
        (skeleton, &params[start as usize..end as usize])
    }

    /// Whether text `id` is `text`.
    fn equals(
        skeletons: &Skeletons,
        texts: &[(u32, u32)],
        params: &[Param],
        id: u32,
        text: &str,
    ) -> bool {
        let (skeleton, held) = Self::text_of(texts, params, id);
        let mut held = held.iter();
        let mut rest = text.as_bytes();
        let mut digits = [0; 10];
        for item in skeletons.items(skeleton) {
            let piece = match item {
                Some(phrase) => phrase.as_bytes(),
                None => decimal(number(*held.next().expect("a param")), &mut digits),
            };
            match rest.strip_prefix(piece) {
                Some(after) => rest = after,
                None => return false,
            }
        }
        rest.is_empty()
    }

    /// Appends the phrases of `text` (no hole in it) to `code`.
    fn phrases(&mut self, text: &str, code: &mut Vec<u8>) {
        let bytes = text.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            let end = phrase_end(bytes, start);
            let phrase = &text[start..end];
            let hashed = hash(phrase.as_bytes());
            let skeletons = &mut self.skeletons;
            let id = match self
                .phrase_index
                .find(hashed, |&id| skeletons.phrase(id as usize) == phrase)
            {
                Some(&id) => id,
                None => {
                    let id = skeletons.phrase_ends.len() as u32;
                    skeletons.phrases.push_str(phrase);
                    skeletons.phrase_ends.push(skeletons.phrases.len() as u32);
                    let skeletons = &*skeletons;
                    self.phrase_index.insert_unique(hashed, id, |&id| {
                        hash(skeletons.phrase(id as usize).as_bytes())
                    });
                    id
                }
            };
            varint(id as usize + 1, code);
            start = end;
        }
    }

    /// The skeleton of `code`.
    fn skeleton(&mut self, code: &[u8]) -> u32 {
        let hashed = hash(code);
        let skeletons = &mut self.skeletons;
        if let Some(&id) = self
            .skeleton_index
            .find(hashed, |&id| skeletons.code(id as usize) == code)
        {
            return id;
        }
        let id = skeletons.code_ends.len() as u32;
        skeletons.codes.extend_from_slice(code);
        skeletons.code_ends.push(skeletons.codes.len() as u32);
        let skeletons = &*skeletons;
        self.skeleton_index
            .insert_unique(hashed, id, |&id| hash(skeletons.code(id as usize)));
        id
    }

    /// Text `id`'s skeleton and the params of its holes.
    pub(super) fn text(&self, id: u32) -> (u32, &[Param]) {
        Self::text_of(&self.texts, &self.params, id)
    }

    #[cfg(test)]
    pub(super) fn skeletons(&self) -> &Skeletons {
        &self.skeletons
    }

    /// How many distinct texts there are, and their bytes.
    pub(super) fn census(&self) -> (usize, usize) {
        (self.texts.len(), self.bytes)
    }

    /// The skeletons, once every text is stored.
    pub(super) fn finish(self) -> Skeletons {
        let Self { mut skeletons, .. } = self;
        skeletons.phrases.shrink_to_fit();
        skeletons.phrase_ends.shrink_to_fit();
        skeletons.codes.shrink_to_fit();
        skeletons.code_ends.shrink_to_fit();
        skeletons
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(texts: &Texts, id: u32) -> String {
        let (skeleton, params) = texts.text(id);
        let mut params = params.iter();
        let mut out = String::new();
        texts.skeletons().write(
            skeleton,
            || number(*params.next().expect("a param")),
            &mut out,
        );
        out
    }

    #[test]
    fn a_text_is_written_back_as_it_was() {
        let samples = [
            "INSERT INTO c12_t3 SELECT f.k FROM f0_1 f WHERE f.file IN (SELECT file FROM runs WHERE run = 12)",
            "SELECT ek FROM pattern WHERE term = 407 AND ek IN (SELECT ek FROM e)",
            "SELECT 1 FROM roots WHERE run = -3 AND s2_f0_1.k = 007 AND c0_x = e0_a AND term = 0",
            "WITH p AS MATERIALIZED (SELECT 1) SELECT * FROM stage_4 ORDER BY 1",
            "term = 1234567890 run = 01",
            "",
        ];
        let mut texts = Texts::default();
        let ids: Vec<u32> = samples.iter().map(|text| texts.intern(text)).collect();
        for (id, text) in ids.iter().zip(samples) {
            assert_eq!(written(&texts, *id), text);
            assert_eq!(texts.intern(text), *id);
        }
    }

    #[test]
    fn texts_differing_in_their_own_numbers_share_a_skeleton() {
        let mut texts = Texts::default();
        let a = texts.intern("DELETE FROM c3_t1 WHERE term = 5 AND run = 3");
        let b = texts.intern("DELETE FROM c4_t1 WHERE term = 9 AND run = 4");
        let c = texts.intern("DELETE FROM c4_t1 WHERE term = 9 AND run = 5");
        assert_ne!(a, b);
        let (skeleton_a, params_a) = texts.text(a);
        let (skeleton_b, params_b) = texts.text(b);
        assert_eq!(skeleton_a, skeleton_b);
        assert_eq!(
            params_a.iter().map(|&p| number(p)).collect::<Vec<_>>(),
            [3, 5, 3]
        );
        assert_eq!(
            params_b.iter().map(|&p| number(p)).collect::<Vec<_>>(),
            [4, 9, 4]
        );
        assert_eq!(texts.text(c).0, skeleton_a);
        assert_eq!(texts.census().0, 3);
    }
}
