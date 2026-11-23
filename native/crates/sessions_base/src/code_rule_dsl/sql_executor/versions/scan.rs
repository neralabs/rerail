//! Walk a frozen range once instead of finding its rank again for every projected cell.
use super::super::super::facts::vtab::{ReadRow, RowStream};
use super::*;

struct Scan {
    stack: Vec<Arc<Node<Row>>>,
    remaining: usize,
    reverse: bool,
}
impl RowStream for Scan {
    fn next(&mut self) -> Option<ReadRow> {
        if self.remaining == 0 {
            return None;
        }
        let n = self.stack.pop()?;
        self.remaining -= 1;
        let mut branch = if self.reverse {
            n.left.clone()
        } else {
            n.right.clone()
        };
        while let Some(node) = branch {
            branch = if self.reverse {
                node.right.clone()
            } else {
                node.left.clone()
            };
            self.stack.push(node);
        }
        Some(ReadRow {
            id: n.value.id,
            cells: n.value.values.clone(),
        })
    }
}
impl Version {
    pub(super) fn stream(
        &self,
        range: std::ops::Range<usize>,
        reverse: bool,
    ) -> Box<dyn RowStream> {
        let mut out = Scan {
            stack: vec![],
            remaining: range.len(),
            reverse,
        };
        if !range.is_empty() {
            let mut rank = if reverse { range.end - 1 } else { range.start };
            let mut at = self.full.clone();
            while let Some(n) = at {
                let left = tree::size(&n.left);
                if rank == left {
                    out.stack.push(n);
                    break;
                }
                if rank < left {
                    at = n.left.clone();
                    if !reverse {
                        out.stack.push(n);
                    }
                } else {
                    rank -= left + 1;
                    at = n.right.clone();
                    if reverse {
                        out.stack.push(n);
                    }
                }
            }
        }
        Box::new(out)
    }
}
