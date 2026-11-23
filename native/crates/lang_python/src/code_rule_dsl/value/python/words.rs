//! Python 3.13 / Unicode 15.1 maximal word runs (str.isalnum plus underscore).
//! Ranges generated offline from CPython 3.13. No interpreter executes in the engine.
pub(crate) fn is_word(cp: u32) -> bool {
    let i = RANGES.partition_point(|&(_, end)| end < cp);
    RANGES.get(i).is_some_and(|&(start, _)| start <= cp)
}

pub(crate) fn delimit(body: &str, raw: bool) -> String {
    let points = if raw { Some(body.chars().map(u32::from).collect()) }
        else { super::decode_escapes(body, false) };
    points.unwrap_or_default().into_iter().map(|cp| {
        if is_word(cp) { char::from_u32(cp).expect("word ranges exclude surrogates") }
        else { '\0' }
    }).collect()
}

// Sorted inclusive ranges; surrogates and punctuation are absent.
include!("words.generated.rs");
