//! Self-contained capture transport. No cell refers to a mutable environment or frame.
//! Lists encode a file and the exact ordered node sequence; NULL means absent.
use rusqlite::{
    Connection, Error, Result,
    functions::FunctionFlags,
    types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef},
};

/// Explicit admission bound: at most 262,141 node IDs; larger lists return an error.
pub(in crate::code_rule_dsl::sql) const MAX_LIST_BYTES: usize = 1024 * 1024;
const HEADER: usize = 12;
const MAGIC: &[u8; 4] = b"RRL1";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl::sql) enum Cell {
    Null,
    Integer(i64),
    List(Vec<u8>),
    Span(Vec<u8>),
}
impl Cell {
    pub(in crate::code_rule_dsl::sql) fn single_node(&self) -> Result<Option<u32>> {
        match self {
            Self::Null => Ok(None),
            Self::Integer(node) => u32::try_from(*node)
                .map(Some)
                .map_err(|_| invalid("invalid node")),
            Self::Span(bytes) => Ok(super::span_capture::decode(bytes)?.node),
            Self::List(_) => Err(invalid("list is not a single capture")),
        }
    }
    pub(in crate::code_rule_dsl::sql) fn memo_key(&self) -> Result<Self> {
        if let Self::Span(bytes) = self {
            let mut span = super::span_capture::decode(bytes)?;
            if span.node.is_some() {
                span.text = None;
            }
            return Ok(Self::Span(super::span_capture::encode(&span)?));
        }
        Ok(self.clone())
    }
    pub(in crate::code_rule_dsl::sql) fn integer(&self) -> Option<i64> {
        if let Self::Integer(v) = self {
            Some(*v)
        } else {
            None
        }
    }
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("capture cell: {message}"))
}

/// Encode a present list, including an empty one. Nodes remain in native order.
pub(in crate::code_rule_dsl::sql) fn encode_list(file: u32, nodes: &[u32]) -> Result<Vec<u8>> {
    if nodes.len() > (MAX_LIST_BYTES - HEADER) / 4 {
        return Err(invalid("list exceeds byte limit"));
    }
    let mut bytes = Vec::with_capacity(HEADER + nodes.len() * 4);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&file.to_le_bytes());
    bytes.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for node in nodes {
        bytes.extend_from_slice(&node.to_le_bytes());
    }
    Ok(bytes)
}
fn parts(bytes: &[u8]) -> Result<(u32, impl Iterator<Item = u32> + '_)> {
    if bytes.len() < HEADER || bytes.len() > MAX_LIST_BYTES || &bytes[..4] != MAGIC {
        return Err(invalid("invalid list header or size"));
    }
    let file = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if count != (bytes.len() - HEADER) / 4 || !(bytes.len() - HEADER).is_multiple_of(4) {
        return Err(invalid("invalid list length"));
    }
    Ok((
        file,
        bytes[HEADER..]
            .chunks_exact(4)
            .map(|n| u32::from_le_bytes(n.try_into().unwrap())),
    ))
}
/// Decode at an AST boundary, rejecting another file and out-of-range nodes.
pub(in crate::code_rule_dsl::sql) fn decode_list(
    bytes: &[u8],
    file: u32,
    node_count: u32,
) -> Result<Vec<u32>> {
    let (encoded_file, nodes) = parts(bytes)?;
    if encoded_file != file {
        return Err(invalid("list belongs to another file"));
    }
    nodes
        .map(|node| {
            if node < node_count {
                Ok(node)
            } else {
                Err(invalid("list node is outside the file"))
            }
        })
        .collect()
}
impl FromSql for Cell {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Null => Ok(Self::Null),
            ValueRef::Integer(v) => Ok(Self::Integer(v)),
            ValueRef::Blob(bytes) if super::span_capture::is_span(bytes) => {
                let _ = super::span_capture::decode(bytes)
                    .map_err(|e| FromSqlError::Other(Box::new(e)))?;
                Ok(Self::Span(bytes.to_vec()))
            }
            ValueRef::Blob(bytes) => {
                let _ = parts(bytes).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                Ok(Self::List(bytes.to_vec()))
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}
impl ToSql for Cell {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(v) => ValueRef::Integer(*v),
            Self::List(v) | Self::Span(v) => ValueRef::Blob(v),
        }))
    }
}
/// Publication only: validates the file and canonical bytes, returns JSON text.
/// AST bounds are validated by decode_list at the pattern input/output boundary.
pub(in crate::code_rule_dsl::sql) fn register(db: &Connection) -> Result<()> {
    db.create_scalar_function(
        "relation_list_json",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let bytes = ctx.get::<Option<Vec<u8>>>(0)?;
            let Some(bytes) = bytes else {
                return Ok(None::<String>);
            };
            let file = ctx.get::<u32>(1)?;
            let (encoded_file, nodes) = parts(&bytes)?;
            if encoded_file != file {
                return Err(invalid("list belongs to another file"));
            }
            let nodes: Vec<u32> = nodes.collect();
            Ok(Some(
                serde_json::to_string(&nodes).map_err(|e| invalid(&e.to_string()))?,
            ))
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_lists_validate_types_files_lengths_and_bounds() {
        let empty = encode_list(2, &[]).unwrap();
        assert_eq!(decode_list(&empty, 2, 0).unwrap(), Vec::<u32>::new());
        let list = encode_list(2, &[3, 1, 3]).unwrap();
        assert_eq!(decode_list(&list, 2, 4).unwrap(), vec![3, 1, 3]);
        assert!(decode_list(&list, 1, 4).is_err());
        assert!(decode_list(&list, 2, 3).is_err());
        assert!(decode_list(&list[..list.len() - 1], 2, 4).is_err());
        assert!(Cell::column_result(ValueRef::Text(b"[]")).is_err());
        assert!(encode_list(0, &vec![0; (MAX_LIST_BYTES - HEADER) / 4 + 1]).is_err());
        assert!(decode_list(b"RRL0xxxxxxxx", 2, 4).is_err());
        assert_ne!(Cell::Null, Cell::List(empty.clone()));
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let value: String = db.query_row("SELECT json_array(json(relation_list_json(?1,2)),json(relation_list_json(NULL,2)))", [&empty], |r|r.get(0)).unwrap();
        assert_eq!(value, "[[],null]");
    }
}
