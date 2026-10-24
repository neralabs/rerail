//! Self-contained single captures exported from a parsed document.
//! The file is the coordinate space of the range; text is the decoded capture text.
use rusqlite::{Error, Result};
use std::ops::Range;

const MAGIC: &[u8; 4] = b"RRS1";
const HEADER: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub file: u32,
    pub node: Option<u32>,
    pub text: Option<String>,
    pub range: Range<usize>,
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("span capture: {message}"))
}
pub(super) fn encode(span: &Span) -> Result<Vec<u8>> {
    if span.range.start > span.range.end {
        return Err(invalid("reversed range"));
    }
    let start = u64::try_from(span.range.start).map_err(|_| invalid("range overflow"))?;
    let end = u64::try_from(span.range.end).map_err(|_| invalid("range overflow"))?;
    let mut bytes = Vec::with_capacity(HEADER + span.text.as_ref().map_or(0, String::len));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&span.file.to_le_bytes());
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(&end.to_le_bytes());
    bytes.extend_from_slice(&span.node.unwrap_or(u32::MAX).to_le_bytes());
    bytes.extend_from_slice(&u32::from(span.text.is_some()).to_le_bytes());
    if let Some(text) = &span.text {
        bytes.extend_from_slice(text.as_bytes());
    }
    Ok(bytes)
}
pub(super) fn is_span(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Span> {
    if bytes.len() < HEADER || !is_span(bytes) {
        return Err(invalid("invalid header"));
    }
    let file = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let start = usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
        .map_err(|_| invalid("range overflow"))?;
    let end = usize::try_from(u64::from_le_bytes(bytes[16..24].try_into().unwrap()))
        .map_err(|_| invalid("range overflow"))?;
    if start > end {
        return Err(invalid("reversed range"));
    }
    let node = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let node = (node != u32::MAX).then_some(node);
    let present = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    let text = match present {
        0 if bytes.len() == HEADER => None,
        1 => Some(
            std::str::from_utf8(&bytes[HEADER..])
                .map_err(|_| invalid("text is not UTF-8"))?
                .to_owned(),
        ),
        _ => return Err(invalid("invalid text presence flag")),
    };
    Ok(Span {
        file,
        node,
        text,
        range: start..end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spans_keep_decoded_text_and_document_coordinates() {
        let span = Span {
            file: 7,
            node: None,
            text: Some("é\0\\x79".into()),
            range: 19..27,
        };
        let bytes = encode(&span).unwrap();
        assert_eq!(decode(&bytes).unwrap(), span);
        assert!(decode(&bytes[..HEADER - 1]).is_err());
        let mut reversed = bytes.clone();
        reversed[8..16].copy_from_slice(&30_u64.to_le_bytes());
        assert!(decode(&reversed).is_err());
        let mut invalid_utf8 = bytes;
        invalid_utf8.push(255);
        assert!(decode(&invalid_utf8).is_err());
    }
}
