//! The parts a shape's class is made of (the join lab's design, step 4), and the classes registered.
//!
//! A class is a shape's join written from parts, one a stage and `Emit` at its end. `part` names the part a stage
//! needs, from its kind, its mode and its input as the catalogue gives them (`shapes.yaml`); `BUILT` lists the parts
//! the library has, and `CLASSES` the generated classes (`register!`'s table, by shape key). A shape with a stage
//! whose part is not built has no class: the lab lists it with that part (`missing`), and its work is the part's
//! claim on the work queue.

/// The parts the library has built.
pub const BUILT: &[&str] = &[];

/// A class registered for a shape: its key and its name (`s-<key>`).
#[derive(Clone, Copy, Debug)]
pub struct Class {
    pub key: u64,
    pub name: &'static str,
}

/// The generated classes, by key (the generator's `table.rs`).
pub const CLASSES: &[Class] = &[];

/// The class of the shape of key `key`.
pub fn class_of(key: u64) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.key == key)
}

/// The key of a shape named `s-<key>`.
pub fn key_of(name: &str) -> Option<u64> {
    u64::from_str_radix(name.strip_prefix("s-")?, 16).ok()
}

/// A view's name by its number in a shape (`Input::View`'s: its read's, `device_chunk::read_kind`: 1 + its place in
/// `view_reads::VIEWS`).
pub fn view_name(view: u32) -> &'static str {
    match view {
        1 => "node",
        2 => "child",
        3 => "field",
        4 => "core",
        5 => "line",
        6 => "loc_line",
        7 => "loc_end",
        8 => "loc_col",
        9 => "docof",
        10 => "tape",
        11 => "depth",
        12 => "order",
        13 => "op",
        14 => "doc",
        _ => "view",
    }
}

/// The part a stage needs: its `kind` and `mode` as the catalogue writes them, its input's kind (`derived`, `view`,
/// `own`, `axis`, `words`, `none`) and view, and whether it is probed by a key. None for a stage that needs no part of
/// its own (a pass).
pub fn part(kind: &str, mode: &str, input: &str, view: Option<u32>, keyed: bool) -> Option<String> {
    let v = || view_name(view.unwrap_or(0));
    Some(match (kind, mode, input) {
        ("pass", ..) => return None,
        ("own", ..) => "Seeded".into(),
        ("map", ..) => "Arith".into(),
        ("filter", ..) => "Cmp".into(),
        ("witness", _, i) => format!("Witness<{}>", if i == "view" { v() } else { "Rel" }),
        ("rowtext" | "names" | "textop" | "verdict", ..) => format!("Text<{kind}>"),
        (_, "agg", _) => "Lattice".into(),
        (_, "semi", "derived") => "Hash".into(),
        (_, "anti", "derived") => "Hash<NEG>".into(),
        (_, "semi", "view") => format!("OwnRead<{}>", v()),
        (_, "anti", "view") => format!("OwnRead<{},NEG>", v()),
        (_, _, "derived") if keyed => "ByValue<Rel>".into(),
        (_, _, "derived") => "Scan<Rel>".into(),
        (_, _, "view") if !keyed => format!("ScanKind<{}>", v()),
        (_, _, "view") if view == Some(2) => "Children<child>".into(),
        (_, _, "view") => format!("OwnRead<{}>", v()),
        (_, _, "axis") => "Climb".into(),
        _ => format!("{kind}/{mode}/{input}"),
    })
}

/// Of `needed` (a shape's parts, `Emit` with them), the first the library has not built.
pub fn missing<'a>(needed: &'a [String]) -> Option<&'a str> {
    needed.iter().map(String::as_str).find(|p| !BUILT.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_stage_names_its_part_by_its_form_alone() {
        assert_eq!(part("atom", "join", "derived", None, false).as_deref(), Some("Scan<Rel>"));
        assert_eq!(part("atom", "join", "derived", None, true).as_deref(), Some("ByValue<Rel>"));
        assert_eq!(part("atom", "anti", "derived", None, true).as_deref(), Some("Hash<NEG>"));
        assert_eq!(part("atom", "join", "view", Some(4), true).as_deref(), Some("OwnRead<core>"));
        assert_eq!(part("atom", "join", "view", Some(2), true).as_deref(), Some("Children<child>"));
        assert_eq!(part("atom", "join", "view", Some(1), false).as_deref(), Some("ScanKind<node>"));
        assert_eq!(part("names", "rowtext", "none", None, false).as_deref(), Some("Text<names>"));
        assert_eq!(part("pass", "join", "none", None, false), None);
        // None built yet: a shape's first part is its missing one.
        let needed = vec!["Scan<Rel>".to_owned(), "Emit".to_owned()];
        assert_eq!(missing(&needed), Some("Scan<Rel>"));
        assert_eq!(key_of("s-28c54b9d365aab6f"), Some(0x28c5_4b9d_365a_ab6f));
        assert!(class_of(0x28c5_4b9d_365a_ab6f).is_none());
    }
}
