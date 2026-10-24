//! Immutable typed capture snapshots. SQL owns control, rollback, and lifetimes.
//! A snapshot update copies seven trie edges, never an entire capture vector.
use super::capture_cell::Cell;
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

type Root = Option<Arc<Node>>;
enum Node {
    Branch(Root, Root),
    Leaf(Cell),
}
fn get(root: &Root, slot: u8, depth: i8) -> Cell {
    match root.as_deref() {
        None => Cell::Null,
        Some(Node::Leaf(cell)) => cell.clone(),
        Some(Node::Branch(left, right)) => get(
            if slot & (1 << depth) == 0 {
                left
            } else {
                right
            },
            slot,
            depth - 1,
        ),
    }
}
fn put(root: &Root, slot: u8, depth: i8, value: &Cell) -> Root {
    if depth < 0 {
        return (value != &Cell::Null).then(|| Arc::new(Node::Leaf(value.clone())));
    }
    let (mut left, mut right) = match root.as_deref() {
        Some(Node::Branch(left, right)) => (left.clone(), right.clone()),
        _ => (None, None),
    };
    let branch = if slot & (1 << depth) == 0 {
        &mut left
    } else {
        &mut right
    };
    *branch = put(branch, slot, depth - 1, value);
    (left.is_some() || right.is_some()).then(|| Arc::new(Node::Branch(left, right)))
}
fn project(root: &Root, mask: u128, depth: i8) -> Root {
    if mask == 0 || root.is_none() {
        return None;
    }
    let width = 1_u32 << (depth + 1);
    let whole = if width == 128 {
        u128::MAX
    } else {
        (1_u128 << width) - 1
    };
    if mask == whole || depth < 0 {
        return root.clone();
    }
    let Some(Node::Branch(left, right)) = root.as_deref() else {
        return root.clone();
    };
    let half = width / 2;
    let selected_left = project(left, mask & ((1_u128 << half) - 1), depth - 1);
    let selected_right = project(right, mask >> half, depth - 1);
    let same = |a: &Root, b: &Root| match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    };
    if same(left, &selected_left) && same(right, &selected_right) {
        return root.clone();
    }
    (selected_left.is_some() || selected_right.is_some())
        .then(|| Arc::new(Node::Branch(selected_left, selected_right)))
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("capture state: {message}"))
}
#[derive(Default)]
struct Store {
    next: i64,
    scopes: BTreeMap<i64, BTreeSet<i64>>,
    states: BTreeMap<i64, (i64, Root)>,
    peak: usize,
    scope_peak: usize,
}
impl Store {
    fn id(&mut self) -> Result<i64> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("handle overflow"))?;
        Ok(self.next)
    }
    fn root(&self, handle: i64) -> Result<Root> {
        if handle == 0 {
            Ok(None)
        } else {
            self.states
                .get(&handle)
                .map(|(_, root)| root.clone())
                .ok_or_else(|| invalid("missing snapshot"))
        }
    }
    fn drop_state(&mut self, handle: i64) -> Result<()> {
        if handle != 0 {
            let (scope, _) = self
                .states
                .remove(&handle)
                .ok_or_else(|| invalid("missing snapshot"))?;
            self.scopes
                .get_mut(&scope)
                .ok_or_else(|| invalid("missing owner"))?
                .remove(&handle);
        }
        Ok(())
    }
    fn update(
        &mut self,
        scope: i64,
        base: i64,
        slot: u8,
        value: Cell,
        consume: bool,
    ) -> Result<i64> {
        if !self.scopes.contains_key(&scope) {
            return Err(invalid("missing scope"));
        }
        if consume
            && base != 0
            && self
                .states
                .get(&base)
                .is_none_or(|(owner, _)| *owner != scope)
        {
            return Err(invalid(
                "owned update requires a snapshot from its invocation",
            ));
        }
        let root = self.root(base)?;
        let root = if get(&root, slot, 6) == value {
            root
        } else {
            put(&root, slot, 6, &value)
        };
        // The immutable root now owns its shared edges independently of base.
        // SQL may consume only an exclusive intermediate, never a saved frame.
        if consume {
            self.drop_state(base)?;
        }
        if root.is_none() {
            return Ok(0);
        }
        let handle = self.id()?;
        self.states.insert(handle, (scope, root));
        self.scopes.get_mut(&scope).unwrap().insert(handle);
        self.peak = self.peak.max(self.states.len());
        Ok(handle)
    }
}
fn slot(value: i64) -> Result<u8> {
    u8::try_from(value)
        .ok()
        .filter(|v| *v < 128)
        .ok_or_else(|| invalid("slot outside existing 128-cell bound"))
}
pub(super) fn register(db: &Connection) -> Result<()> {
    let store = Arc::new(Mutex::new(Store::default()));
    let flags = FunctionFlags::SQLITE_UTF8;
    let begin = Arc::clone(&store);
    db.create_scalar_function("relation_state_begin", 0, flags, move |_| {
        let mut store = begin.lock().unwrap();
        let scope = store.id()?;
        store.scopes.insert(scope, BTreeSet::new());
        store.scope_peak = store.scope_peak.max(store.scopes.len());
        Ok(scope)
    })?;
    let read = Arc::clone(&store);
    db.create_scalar_function("relation_state_get", 2, flags, move |ctx| {
        let Some(handle) = ctx.get::<Option<i64>>(0)? else {
            return Ok(Cell::Null);
        };
        let slot = slot(ctx.get(1)?)?;
        Ok(get(&read.lock().unwrap().root(handle)?, slot, 6))
    })?;
    let update = Arc::clone(&store);
    db.create_scalar_function("relation_state_put", 4, flags, move |ctx| {
        let Some(scope) = ctx.get::<Option<i64>>(0)? else {
            return Ok(None);
        };
        let base = ctx.get::<i64>(1)?;
        let slot = slot(ctx.get(2)?)?;
        let value = ctx.get::<Cell>(3)?;
        update
            .lock()
            .unwrap()
            .update(scope, base, slot, value, false)
            .map(Some)
    })?;
    let owned = Arc::clone(&store);
    db.create_scalar_function("relation_state_put_owned", 4, flags, move |ctx| {
        let Some(scope) = ctx.get::<Option<i64>>(0)? else {
            return Ok(None);
        };
        owned
            .lock()
            .unwrap()
            .update(scope, ctx.get(1)?, slot(ctx.get(2)?)?, ctx.get(3)?, true)
            .map(Some)
    })?;
    let projection = Arc::clone(&store);
    db.create_scalar_function("relation_state_project", 4, flags, move |ctx| {
        let Some(scope) = ctx.get::<Option<i64>>(0)? else {
            return Ok(None);
        };
        let base = ctx.get::<i64>(1)?;
        let low = ctx.get::<i64>(2)? as u64;
        let high = ctx.get::<i64>(3)? as u64;
        let mask = u128::from(low) | (u128::from(high) << 64);
        let mut store = projection.lock().unwrap();
        if !store.scopes.contains_key(&scope) {
            return Err(invalid("missing scope"));
        }
        let root = project(&store.root(base)?, mask, 6);
        if root.is_none() {
            return Ok(Some(0));
        }
        let handle = store.id()?;
        store.states.insert(handle, (scope, root));
        store.scopes.get_mut(&scope).unwrap().insert(handle);
        store.peak = store.peak.max(store.states.len());
        Ok(Some(handle))
    })?;
    let release = Arc::clone(&store);
    db.create_scalar_function("relation_state_drop", 1, flags, move |ctx| {
        if let Some(handle) = ctx.get::<Option<i64>>(0)? {
            release.lock().unwrap().drop_state(handle)?;
        }
        Ok(1)
    })?;
    let finish = Arc::clone(&store);
    db.create_scalar_function("relation_state_end", 1, flags, move |ctx| {
        if let Some(scope) = ctx.get::<Option<i64>>(0)? {
            let mut store = finish.lock().unwrap();
            let handles = store
                .scopes
                .remove(&scope)
                .ok_or_else(|| invalid("missing scope"))?;
            for handle in handles {
                store.states.remove(&handle);
            }
        }
        Ok(1)
    })?;
    let live = Arc::clone(&store);
    db.create_scalar_function("relation_state_rows", 0, flags, move |_| {
        Ok(live.lock().unwrap().states.len() as i64)
    })?;
    let peak = Arc::clone(&store);
    db.create_scalar_function("relation_state_peak", 0, flags, move |_| {
        Ok(peak.lock().unwrap().peak as i64)
    })?;
    let scopes = Arc::clone(&store);
    db.create_scalar_function("relation_state_scopes", 0, flags, move |_| {
        Ok(scopes.lock().unwrap().scopes.len() as i64)
    })?;
    db.create_scalar_function("relation_state_scope_peak", 0, flags, move |_| {
        Ok(store.lock().unwrap().scope_peak as i64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_share_unchanged_paths_and_rollback_all_typed_cells() {
        let mut a = None;
        a = put(&a, 0, 6, &Cell::Integer(7));
        let b = put(
            &a,
            127,
            6,
            &Cell::List(super::super::capture_cell::encode_list(3, &[8, 2]).unwrap()),
        );
        let (Some(Node::Branch(a_left, _)), Some(Node::Branch(b_left, _))) =
            (a.as_deref(), b.as_deref())
        else {
            panic!("branches");
        };
        assert!(Arc::ptr_eq(
            a_left.as_ref().unwrap(),
            b_left.as_ref().unwrap()
        ));
        assert_eq!(get(&a, 127, 6), Cell::Null);
        assert_eq!(get(&b, 0, 6), Cell::Integer(7));
        let c = put(&b, 0, 6, &Cell::Null);
        assert_eq!(get(&c, 0, 6), Cell::Null);
        assert_eq!(get(&b, 0, 6), Cell::Integer(7));
    }
    #[test]
    fn nested_scope_cleanup_does_not_destroy_derived_or_outer_snapshots() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let scalar = |sql: &str| db.query_row::<i64, _, _>(sql, [], |r| r.get(0)).unwrap();
        let outer = scalar("SELECT relation_state_begin()");
        let a = scalar(&format!("SELECT relation_state_put({outer},0,4,31)"));
        let inner = scalar("SELECT relation_state_begin()");
        let b = scalar(&format!("SELECT relation_state_put({inner},{a},4,42)"));
        assert_eq!(scalar(&format!("SELECT relation_state_get({a},4)")), 31);
        assert_eq!(scalar(&format!("SELECT relation_state_get({b},4)")), 42);
        scalar(&format!("SELECT relation_state_end({inner})"));
        assert_eq!(scalar("SELECT relation_state_rows()"), 1);
        assert_eq!(scalar(&format!("SELECT relation_state_get({a},4)")), 31);
        assert!(
            db.query_row::<i64, _, _>(&format!("SELECT relation_state_get({b},4)"), [], |r| r
                .get(0))
                .is_err()
        );
        scalar(&format!("SELECT relation_state_end({outer})"));
        assert_eq!(scalar("SELECT relation_state_rows()"), 0);
        assert!(
            db.query_row::<i64, _, _>("SELECT relation_state_get(0,128)", [], |r| r.get(0))
                .is_err()
        );
    }

    #[test]
    fn sql_roundtrips_list_and_shadow_span_without_interpreting_payloads() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let scope: i64 = db
            .query_row("SELECT relation_state_begin()", [], |r| r.get(0))
            .unwrap();
        let values = [
            Cell::List(super::super::capture_cell::encode_list(2, &[9, 3, 9]).unwrap()),
            Cell::Span(
                super::super::span_capture::encode(&super::super::span_capture::Span {
                    file: 2,
                    node: Some(7),
                    text: Some("hidden".into()),
                    range: 10..16,
                })
                .unwrap(),
            ),
            Cell::Null,
            Cell::Integer(-7),
        ];
        let mut base = 0;
        for (slot, value) in values.iter().enumerate() {
            let next: i64 = db
                .query_row(
                    "SELECT relation_state_put(?1,?2,?3,?4)",
                    rusqlite::params![scope, base, slot as i64, value],
                    |r| r.get(0),
                )
                .unwrap();
            let actual: Cell = db
                .query_row(
                    "SELECT relation_state_get(?1,?2)",
                    [next, slot as i64],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(&actual, value);
            if base != 0 {
                db.query_row::<i64, _, _>("SELECT relation_state_drop(?1)", [base], |r| r.get(0))
                    .unwrap();
            }
            base = next;
        }
        let first: Cell = db
            .query_row("SELECT relation_state_get(?1,0)", [base], |r| r.get(0))
            .unwrap();
        assert_eq!(first, values[0]);
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_state_rows()", [], |r| r.get(0))
                .unwrap(),
            1
        );
        db.query_row::<i64, _, _>("SELECT relation_state_end(?1)", [scope], |r| r.get(0))
            .unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_state_rows()", [], |r| r.get(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn projection_shares_retained_paths_and_empty_masks_use_zero() {
        let base = put(
            &put(&None, 0, 6, &Cell::Integer(11)),
            127,
            6,
            &Cell::Integer(22),
        );
        let selected = project(&base, 1, 6);
        let (Some(Node::Branch(left, _)), Some(Node::Branch(kept, None))) =
            (base.as_deref(), selected.as_deref())
        else {
            panic!("projected branch");
        };
        assert!(Arc::ptr_eq(left.as_ref().unwrap(), kept.as_ref().unwrap()));
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let scalar = |sql: &str| db.query_row::<i64, _, _>(sql, [], |r| r.get(0)).unwrap();
        let owner = scalar("SELECT relation_state_begin()");
        let a = scalar(&format!("SELECT relation_state_put({owner},0,0,11)"));
        let b = scalar(&format!("SELECT relation_state_put({owner},{a},127,22)"));
        let child = scalar("SELECT relation_state_begin()");
        let selected = scalar(&format!(
            "SELECT relation_state_project({child},{b},0,-9223372036854775808)"
        ));
        assert_eq!(
            scalar(&format!("SELECT relation_state_get({selected},127)")),
            22
        );
        let missing: Cell = db
            .query_row("SELECT relation_state_get(?1,0)", [selected], |r| r.get(0))
            .unwrap();
        assert_eq!(missing, Cell::Null);
        assert_eq!(
            scalar(&format!("SELECT relation_state_project({child},{b},0,0)")),
            0
        );
        scalar(&format!("SELECT relation_state_end({owner})"));
        assert_eq!(
            scalar(&format!("SELECT relation_state_get({selected},127)")),
            22
        );
        scalar(&format!("SELECT relation_state_end({child})"));
        assert_eq!(scalar("SELECT relation_state_rows()"), 0);
        assert_eq!(scalar("SELECT relation_state_scopes()"), 0);
    }

    #[test]
    fn consuming_updates_bound_intermediate_handles_without_touching_saved_snapshots() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let scalar = |sql: &str| db.query_row::<i64, _, _>(sql, [], |r| r.get(0)).unwrap();
        let scope = scalar("SELECT relation_state_begin()");
        let saved = scalar(&format!("SELECT relation_state_put({scope},0,0,7)"));
        let mut trial = scalar(&format!("SELECT relation_state_put({scope},{saved},1,8)"));
        for value in 0..128 {
            let prior = trial;
            trial = scalar(&format!(
                "SELECT relation_state_put_owned({scope},{trial},1,{value})"
            ));
            assert!(
                db.query_row::<i64, _, _>("SELECT relation_state_get(?1,1)", [prior], |r| r.get(0))
                    .is_err()
            );
            assert_eq!(scalar("SELECT relation_state_rows()"), 2);
            assert_eq!(scalar(&format!("SELECT relation_state_get({saved},0)")), 7);
        }
        assert_eq!(scalar("SELECT relation_state_peak()"), 2);
        let foreign = scalar("SELECT relation_state_begin()");
        assert!(
            db.query_row::<i64, _, _>(
                &format!("SELECT relation_state_put_owned({foreign},{saved},0,9)"),
                [],
                |r| r.get(0)
            )
            .is_err()
        );
        scalar(&format!("SELECT relation_state_end({foreign})"));
        scalar(&format!("SELECT relation_state_end({scope})"));
        assert_eq!(scalar("SELECT relation_state_rows()"), 0);
    }
}
