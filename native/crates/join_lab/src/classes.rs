//! The classes' generator (`join-lab classes`, the design's step 4): the table of classes
//! (`rerail_ukl_rust::join_program::classes`' `table.rs`) written from the shapes the captures' labels name and the
//! forms their stage records had, merged with the classes the table has.
//!
//! A class is its shape's key and name and the forms of its stages (each `stage::<F>()`, its parts compiled for the
//! form): every form a run of the shape had, as a shape's runs may differ in what the shape's key leaves out (a
//! counter's, a live length's). Each form is one static, which every class with a stage of it names.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Classes by key: each one's name and its stages' forms.
pub type Table = BTreeMap<u64, (String, BTreeSet<u64>)>;

/// Where the table lies in the tree.
pub fn table_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ukl_rust/src/join_program/classes/table.rs")
}

/// The classes `text` (a `table.rs`) holds.
pub fn parse(text: &str) -> Result<Table, String> {
    let mut t = Table::new();
    for line in text.lines().map(str::trim).filter(|l| l.starts_with("Class { key: ")) {
        let bad = || format!("table.rs: {line}");
        let key = line.strip_prefix("Class { key: 0x").and_then(|r| r.split(',').next()).ok_or_else(bad)?;
        let key = u64::from_str_radix(key, 16).map_err(|_| bad())?;
        let name = line.split("name: \"").nth(1).and_then(|r| r.split('"').next()).ok_or_else(bad)?;
        let forms = line.split("stages: &[").nth(1).and_then(|r| r.split(']').next()).ok_or_else(bad)?;
        let forms = forms.split(',').map(str::trim).filter(|f| !f.is_empty()).map(|f| f.strip_prefix("&F_").and_then(|h| u64::from_str_radix(h, 16).ok()).ok_or_else(bad)).collect::<Result<_, _>>()?;
        t.insert(key, (name.to_owned(), forms));
    }
    Ok(t)
}

/// Adds the classes of `waves`' labels to `t`: each shape's, with the forms its runs had. A label with no forms (a
/// capture made before they were named) adds none. Whether `t` grew.
pub fn add(t: &mut Table, waves: &[crate::Wave]) -> bool {
    let mut grew = false;
    for l in waves.iter().flat_map(|w| &w.labels).filter(|l| !l.forms.is_empty()) {
        let Some(key) = rerail_join_shapes::parts::key_of(&l.shape) else { continue };
        let e = t.entry(key).or_insert_with(|| {
            grew = true;
            (l.shape.clone(), BTreeSet::new())
        });
        for &f in &l.forms {
            grew |= e.1.insert(f);
        }
    }
    grew
}

/// The table `t` as `table.rs`.
pub fn write(t: &Table) -> String {
    let mut s = String::from(
        "//! The classes, by shape key: written by `join-lab classes` from the captures' labels (each run's shape and its\n\
         //! stage records' forms), merged with those it held. Not edited by hand: each form is a stage's parts\n\
         //! (`stage::<F>()`), each class its shape's key, name and forms.\n\n\
         #[allow(unused_imports)]\n\
         use super::{Class, StageClass, stage};\n\n\
         pub const CLASSES: &[Class] = &[\n",
    );
    for (key, (name, forms)) in t {
        let stages: Vec<String> = forms.iter().map(|f| format!("&F_{f:016x}")).collect();
        s += &format!("    Class {{ key: 0x{key:016x}, name: \"{name}\", stages: &[{}] }},\n", stages.join(", "));
    }
    s += "];\n";
    let forms: BTreeSet<u64> = t.values().flat_map(|(_, f)| f.iter().copied()).collect();
    if !forms.is_empty() {
        s += "\n";
    }
    for f in forms {
        s += &format!("const F_{f:016x}: StageClass = stage::<0x{f:016x}>();\n");
    }
    s
}

/// Whether every label of `waves` that names forms has its shape's class with each of them in `t`.
pub fn covers(t: &Table, waves: &[crate::Wave]) -> Result<(), String> {
    for l in waves.iter().flat_map(|w| &w.labels).filter(|l| !l.forms.is_empty()) {
        let key = rerail_join_shapes::parts::key_of(&l.shape).ok_or_else(|| format!("{}: no shape key", l.shape))?;
        let Some((_, forms)) = t.get(&key) else { return Err(format!("{}: no class", l.shape)) };
        if let Some(f) = l.forms.iter().find(|f| !forms.contains(f)) {
            return Err(format!("{}: no stage of form {f:016x}", l.shape));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_read_back_as_written() {
        let mut t = Table::new();
        t.insert(0x28c5_4b9d_365a_ab6f, ("s-28c54b9d365aab6f".into(), [0x0100_0000_0000_0200u64, 0x0100_0000_0000_0201].into()));
        t.insert(0x1, ("s-0000000000000001".into(), [0x0100_0000_0000_0200u64].into()));
        let text = write(&t);
        assert_eq!(parse(&text).unwrap(), t);
        // One static a form, however many classes name it.
        assert_eq!(text.matches("const F_0100000000000200").count(), 1);
        // The table in the tree is one it wrote.
        let tree = std::fs::read_to_string(table_path()).unwrap();
        assert_eq!(write(&parse(&tree).unwrap()), tree);
    }
}
