//! AVL summaries for an associative, commutative, idempotent registered lattice merge.
//! SQL owns the nodes. Each edit reads and rewrites only its search and rotation paths.
use super::*;
use rusqlite::params_from_iter;
fn read<T>(
    db: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
    row: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    db.prepare_cached(sql)?.query_row(params, row)
}
fn write(
    db: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<usize> {
    db.prepare_cached(sql)?.execute(params)
}

#[derive(Clone)]
pub(super) struct JoinTree {
    pub nodes: String,
    pub roots: String,
    pub merge: String,
    pub keys: usize,
    pub top: i64,
}
#[derive(Clone, Copy)]
struct Node {
    value: i64,
    left: Option<i64>,
    right: Option<i64>,
    height: i64,
    summary: i64,
}
struct Edit<'a> {
    db: &'a rusqlite::Connection,
    tree: &'a JoinTree,
    key: Vec<i64>,
    cache: BTreeMap<i64, Node>,
}
impl JoinTree {
    pub fn initialize(
        &self,
        db: &rusqlite::Connection,
        heads: &str,
        objects: &mut Vec<(String, bool)>,
    ) -> Result<()> {
        let keys = names("", self.keys);
        owned(
            db,
            objects,
            &self.nodes,
            &format!(
                "{},value INTEGER NOT NULL,left_value INTEGER,right_value INTEGER,height INTEGER NOT NULL,summary INTEGER NOT NULL,PRIMARY KEY({keys},value)",
                schema(self.keys)
            ),
        )?;
        owned(
            db,
            objects,
            &self.roots,
            &format!(
                "{},root_value INTEGER NOT NULL,summary INTEGER NOT NULL,PRIMARY KEY({keys})",
                schema(self.keys)
            ),
        )?;
        let mut stmt = db
            .prepare(&format!(
                "SELECT {} FROM {} ORDER BY {}",
                names("", self.keys + 1),
                quote(heads),
                names("", self.keys + 1)
            ))
            .map_err(error)?;
        let rows = stmt
            .query_map([], |r| {
                (0..self.keys + 1)
                    .map(|c| r.get::<_, i64>(c))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(error)?;
        let mut key = Vec::new();
        let mut values = Vec::new();
        for row in rows {
            let row = row.map_err(error)?;
            if row[..self.keys] != key && !values.is_empty() {
                self.build(db, std::mem::take(&mut key), &values)?;
                values.clear();
            }
            key = row[..self.keys].to_vec();
            values.push(row[self.keys]);
        }
        if !values.is_empty() {
            self.build(db, key, &values)?;
        }
        Ok(())
    }
    fn build(&self, db: &rusqlite::Connection, key: Vec<i64>, values: &[i64]) -> Result<()> {
        let mut edit = Edit {
            db,
            tree: self,
            key,
            cache: BTreeMap::new(),
        };
        let root = edit.build(values)?;
        edit.root(root)
    }
    pub fn apply(&self, db: &rusqlite::Connection, changed: &str) -> Result<()> {
        let cols = names("", self.keys + 1);
        let mut stmt = db
            .prepare(&format!(
                "SELECT {cols},n FROM {changed} WHERE (old_n=0)<>(n=0) ORDER BY {cols}"
            ))
            .map_err(error)?;
        let rows = stmt
            .query_map([], |r| {
                (0..self.keys + 2)
                    .map(|c| r.get::<_, i64>(c))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(error)?;
        let mut edits = BTreeMap::<Vec<i64>, Edit<'_>>::new();
        for row in rows {
            let row = row.map_err(error)?;
            let key = row[..self.keys].to_vec();
            let edit = edits.entry(key.clone()).or_insert_with(|| Edit {
                db,
                tree: self,
                key,
                cache: BTreeMap::new(),
            });
            let root = read(
                db,
                &format!(
                    "SELECT root_value FROM {} WHERE {}",
                    quote(&self.roots),
                    self.where_key()
                ),
                params_from_iter(edit.key.iter()),
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(error)?;
            let next = edit.set(root, row[self.keys], row[self.keys + 1] > 0)?;
            edit.root(next)?;
        }
        Ok(())
    }
    fn where_key(&self) -> String {
        (0..self.keys)
            .map(|i| format!("c{i}=?{}", i + 1))
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}
impl Edit<'_> {
    fn load(&mut self, value: i64) -> Result<Node> {
        if let Some(node) = self.cache.get(&value) {
            return Ok(*node);
        }
        let node = read(
            self.db,
            &format!(
                "SELECT left_value,right_value,height,summary FROM {} WHERE {} AND value=?{}",
                quote(&self.tree.nodes),
                self.tree.where_key(),
                self.key.len() + 1
            ),
            params_from_iter(self.key.iter().copied().chain([value])),
            |r| {
                Ok(Node {
                    value,
                    left: r.get(0)?,
                    right: r.get(1)?,
                    height: r.get(2)?,
                    summary: r.get(3)?,
                })
            },
        )
        .map_err(error)?;
        self.cache.insert(value, node);
        Ok(node)
    }
    fn child(&mut self, value: Option<i64>) -> Result<Option<Node>> {
        value.map(|v| self.load(v)).transpose()
    }
    fn join(&self, a: i64, b: i64) -> Result<i64> {
        if a == self.tree.top || b == self.tree.top {
            return Ok(self.tree.top);
        }
        if a == b {
            return Ok(a);
        }
        read(
            self.db,
            &format!("SELECT {}(?1,?2)", self.tree.merge),
            [a, b],
            |r| r.get(0),
        )
        .map_err(error)
    }
    fn save(&mut self, mut node: Node) -> Result<Node> {
        let left = self.child(node.left)?;
        let right = self.child(node.right)?;
        node.height = 1 + left
            .map_or(0, |n| n.height)
            .max(right.map_or(0, |n| n.height));
        node.summary = node.value;
        if let Some(left) = left {
            node.summary = self.join(left.summary, node.summary)?;
        }
        if let Some(right) = right {
            node.summary = self.join(node.summary, right.summary)?;
        }
        let values = self
            .key
            .iter()
            .copied()
            .map(Some)
            .chain([
                Some(node.value),
                node.left,
                node.right,
                Some(node.height),
                Some(node.summary),
            ])
            .collect::<Vec<_>>();
        let marks = (1..=values.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        write(
            self.db,
            &format!(
                "INSERT OR REPLACE INTO {} VALUES({marks})",
                quote(&self.tree.nodes)
            ),
            params_from_iter(values),
        )
        .map_err(error)?;
        self.cache.insert(node.value, node);
        Ok(node)
    }
    fn build(&mut self, values: &[i64]) -> Result<Option<i64>> {
        if values.is_empty() {
            return Ok(None);
        }
        let mid = values.len() / 2;
        let left = self.build(&values[..mid])?;
        let right = self.build(&values[mid + 1..])?;
        Ok(Some(
            self.save(Node {
                value: values[mid],
                left,
                right,
                height: 1,
                summary: values[mid],
            })?
            .value,
        ))
    }
    fn rotate_left(&mut self, mut node: Node) -> Result<Node> {
        let mut right = self.load(node.right.unwrap())?;
        node.right = right.left;
        let node = self.save(node)?;
        right.left = Some(node.value);
        self.save(right)
    }
    fn rotate_right(&mut self, mut node: Node) -> Result<Node> {
        let mut left = self.load(node.left.unwrap())?;
        node.left = left.right;
        let node = self.save(node)?;
        left.right = Some(node.value);
        self.save(left)
    }
    fn balance(&mut self, mut node: Node) -> Result<Node> {
        node = self.save(node)?;
        let left = self.child(node.left)?;
        let right = self.child(node.right)?;
        let balance = left.map_or(0, |n| n.height) - right.map_or(0, |n| n.height);
        if balance > 1 {
            let left = left.unwrap();
            if self.child(left.left)?.map_or(0, |n| n.height)
                < self.child(left.right)?.map_or(0, |n| n.height)
            {
                node.left = Some(self.rotate_left(left)?.value);
            }
            return self.rotate_right(node);
        }
        if balance < -1 {
            let right = right.unwrap();
            if self.child(right.right)?.map_or(0, |n| n.height)
                < self.child(right.left)?.map_or(0, |n| n.height)
            {
                node.right = Some(self.rotate_right(right)?.value);
            }
            return self.rotate_left(node);
        }
        Ok(node)
    }
    fn set(&mut self, root: Option<i64>, value: i64, present: bool) -> Result<Option<i64>> {
        let Some(root) = root else {
            return if present {
                Ok(Some(
                    self.save(Node {
                        value,
                        left: None,
                        right: None,
                        height: 1,
                        summary: value,
                    })?
                    .value,
                ))
            } else {
                Ok(None)
            };
        };
        let mut node = self.load(root)?;
        if value < root {
            node.left = self.set(node.left, value, present)?;
        } else if value > root {
            node.right = self.set(node.right, value, present)?;
        } else if present {
            return Ok(Some(root));
        } else {
            write(
                self.db,
                &format!(
                    "DELETE FROM {} WHERE {} AND value=?{}",
                    quote(&self.tree.nodes),
                    self.tree.where_key(),
                    self.key.len() + 1
                ),
                params_from_iter(self.key.iter().copied().chain([value])),
            )
            .map_err(error)?;
            self.cache.remove(&value);
            if node.left.is_none() {
                return Ok(node.right);
            }
            if node.right.is_none() {
                return Ok(node.left);
            }
            let mut successor = self.load(node.right.unwrap())?;
            while let Some(left) = successor.left {
                successor = self.load(left)?;
            }
            let right = self.set(node.right, successor.value, false)?;
            node = Node {
                value: successor.value,
                left: node.left,
                right,
                height: 1,
                summary: successor.value,
            };
        }
        Ok(Some(self.balance(node)?.value))
    }
    fn root(&mut self, value: Option<i64>) -> Result<()> {
        if let Some(value) = value {
            let summary = self.load(value)?.summary;
            let marks = (1..=self.key.len() + 2)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            write(
                self.db,
                &format!(
                    "INSERT OR REPLACE INTO {} VALUES({marks})",
                    quote(&self.tree.roots)
                ),
                params_from_iter(self.key.iter().copied().chain([value, summary])),
            )
            .map_err(error)?;
        } else {
            write(
                self.db,
                &format!(
                    "DELETE FROM {} WHERE {}",
                    quote(&self.tree.roots),
                    self.tree.where_key()
                ),
                params_from_iter(self.key.iter()),
            )
            .map_err(error)?;
        }
        Ok(())
    }
}
