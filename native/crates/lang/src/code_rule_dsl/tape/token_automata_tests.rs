use super::*;

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn pairs(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs.iter().map(|&(token, field)| (token.to_owned(), field.to_owned())).collect()
}

/// Two kinds with implied tokens, two with parents, three tokens and an extra.
fn census() -> Census {
    Census {
        implied: BTreeMap::from([("call".to_owned(), pairs(&[("(", ""), (")", "")])), ("list".to_owned(), pairs(&[(",", "")]))]),
        parents: BTreeMap::from([("call".to_owned(), names(&["block", "list"])), ("name".to_owned(), names(&["call"]))]),
        tokens: names(&["(", ")", ","]),
        extras: names(&["comment"]),
    }
}

/// Picks one of a census's lists.
type List = for<'c> fn(&CensusIn<'c>) -> Words<'c>;

/// Where `slice` starts in `bytes`.
fn offset(bytes: &[u8], slice: &[u8]) -> usize {
    slice.as_ptr() as usize - bytes.as_ptr() as usize
}

/// `bytes` with word `of` of entry `at` of the list `list` picks out of the census (as decoded from `bytes`) set to
/// `value`.
fn with_word(bytes: &[u8], list: List, at: usize, of: usize, value: u32) -> Vec<u8> {
    let census = decode_census(bytes).expect("the census decodes");
    let words = list(&census);
    let start = offset(bytes, words.bytes) + 4 * (at * words.width + of);
    let mut out = bytes.to_vec();
    out[start..start + 4].copy_from_slice(&value.to_le_bytes());
    out
}

/// `bytes` with entries `a` and `b` of the list `list` picks out swapped.
fn with_swapped(bytes: &[u8], list: List, a: usize, b: usize) -> Vec<u8> {
    let census = decode_census(bytes).expect("the census decodes");
    let words = list(&census);
    let entry = |at: usize| offset(bytes, words.bytes) + 4 * at * words.width..offset(bytes, words.bytes) + 4 * (at + 1) * words.width;
    let mut out = bytes.to_vec();
    let (first, second) = (bytes[entry(a)].to_vec(), bytes[entry(b)].to_vec());
    out[entry(a)].copy_from_slice(&second);
    out[entry(b)].copy_from_slice(&first);
    out
}

/// `bytes` with entry `from` of the list `list` picks out copied over entry `to`.
fn with_copied(bytes: &[u8], list: List, from: usize, to: usize) -> Vec<u8> {
    let census = decode_census(bytes).expect("the census decodes");
    let words = list(&census);
    let entry = |at: usize| offset(bytes, words.bytes) + 4 * at * words.width..offset(bytes, words.bytes) + 4 * (at + 1) * words.width;
    let mut out = bytes.to_vec();
    out[entry(to)].copy_from_slice(&bytes[entry(from)]);
    out
}

#[test]
fn a_census_reads_back_what_it_was_written_from() {
    let census = census();
    let bytes = encode_census(&census);
    let read = decode_census(&bytes).expect("the census decodes");
    assert_eq!(read.names, census_names(&census));
    let name = |at: u32| read.names[at as usize];
    let pair = |words: Words<'_>, at: usize| (name(words.get(at, 0)), name(words.get(at, 1)));
    assert_eq!((0..read.distinct.len()).map(|at| pair(read.distinct, at)).collect::<Vec<_>>(), [("(", ""), (")", ""), (",", "")]);
    assert_eq!(read.tokens.firsts().map(name).collect::<Vec<_>>(), ["(", ")", ","]);
    assert_eq!(read.extras.firsts().map(name).collect::<Vec<_>>(), ["comment"]);
    let call = read.implied.find(read.names.binary_search(&"call").unwrap() as u32).expect("call has implied tokens");
    let range = read.implied_pairs.slice(read.implied.get(call, 1) as usize, read.implied.get(call, 2) as usize);
    assert_eq!((0..range.len()).map(|at| pair(range, at)).collect::<Vec<_>>(), [("(", ""), (")", "")]);
}

#[test]
fn a_census_whose_bytes_it_did_not_write_is_refused() {
    let census = census();
    let bytes = encode_census(&census);
    let read = decode_census(&bytes).expect("the census decodes");
    let names = read.names.len() as u32;
    let refused = |bytes: &[u8], what: &str| assert!(decode_census(bytes).is_none(), "{what}: decoded");
    // Another format, a byte past the end, a byte short.
    refused(&[], "no bytes");
    let mut other = bytes.clone();
    other[0] ^= 1;
    refused(&other, "another format");
    refused(&[bytes.as_slice(), &[0]].concat(), "a trailing byte");
    refused(&bytes[..bytes.len() - 1], "a byte short");
    // An index that names no name, in each list, and a kind that names none: in the list's last entry, so the
    // lists stay in order and only the bound refuses them.
    for (list, what) in [
        ((|census| census.distinct) as List, "distinct"),
        (|census| census.tokens, "tokens"),
        (|census| census.extras, "extras"),
        (|census| census.implied_pairs, "implied pairs"),
        (|census| census.parent_kinds, "parent kinds"),
        (|census| census.implied, "an implied kind"),
        (|census| census.parents, "a parents kind"),
    ] {
        let last = list(&read).len() - 1;
        assert!(decode_census(&with_word(&bytes, list, last, 0, names - 1)).is_some(), "{what}: the last name is refused");
        refused(&with_word(&bytes, list, last, 0, names), what);
    }
    // A range that runs past its list, by its start and by its length.
    refused(&with_word(&bytes, |census| census.implied, 1, 1, 4), "an implied range's start");
    refused(&with_word(&bytes, |census| census.implied, 0, 2, 4), "an implied range's length");
    refused(&with_word(&bytes, |census| census.parents, 0, 2, u32::MAX), "a parents range's length");
    // Lists a lookup searches, out of order or with an entry twice (they ascend strictly).
    refused(&with_swapped(&bytes, |census| census.tokens, 0, 1), "unsorted tokens");
    refused(&with_swapped(&bytes, |census| census.distinct, 0, 2), "unsorted distinct pairs");
    refused(&with_swapped(&bytes, |census| census.implied, 0, 1), "unsorted implied kinds");
    refused(&with_swapped(&bytes, |census| census.parents, 0, 1), "unsorted parents kinds");
    refused(&with_copied(&bytes, |census| census.tokens, 0, 1), "a token twice");
    refused(&with_copied(&bytes, |census| census.distinct, 0, 1), "a distinct pair twice");
    refused(&with_copied(&bytes, |census| census.implied, 0, 1), "an implied kind twice");
    refused(&with_copied(&bytes, |census| census.parents, 0, 1), "a parents kind twice");
    // Names out of order, and a name twice: `(` and `)` swapped where they lie (each one byte, after its length),
    // and `)` written as `(`.
    let (open, close) = (read.names.binary_search(&"(").unwrap(), read.names.binary_search(&")").unwrap());
    let (open, close) = (offset(&bytes, read.names[open].as_bytes()), offset(&bytes, read.names[close].as_bytes()));
    let mut swapped = bytes.clone();
    swapped.swap(open, close);
    refused(&swapped, "unsorted names");
    let mut twice = bytes.clone();
    twice[close] = b'(';
    refused(&twice, "a name twice");
}
