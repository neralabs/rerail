//! Byte-exact CPython 3.13 AST serialization for signed source fingerprints.
//! This uses Rerail's existing parser. It never evaluates Python source.
use ast_grep_core::{Doc, Node};

#[derive(Debug)]
pub(crate) struct Error(pub String);
pub(crate) struct Function {
    pub body: String,
    pub signature: String,
}

pub(crate) fn dump<D: Doc>(node: &Node<'_, D>) -> Result<String, Error> {
    Err(Error(format!("CPython AST serialization is not implemented for {}", node.kind())))
}
pub(crate) fn function<D: Doc>(node: &Node<'_, D>) -> Result<Function, Error> {
    let _ = node;
    Err(Error("CPython function fingerprints are not implemented".into()))
}

#[cfg(test)]
mod tests;
