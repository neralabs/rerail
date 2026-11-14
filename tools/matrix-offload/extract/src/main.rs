//! Extracts the matrix-offload workloads from a Python corpus (one file per line on stdin, paths under ROOT).
//!
//! Writes, under OUT (all integers little-endian):
//! - `files.txt`: one path per document, in document order.
//! - `names.txt`: the identifier vocabulary, one name per line, by name id.
//! - `doc_names.bin`: per document, the sorted unique name ids it mentions (a CSR: u32 n, u64 offsets[n+1], u32 ids).
//! - `occ.bin`: per document, every identifier occurrence: u32 n, u64 offsets[n+1], u32 name[], u8 role[]
//!   (role 1 = binding: a def/class name, parameter, assignment/for/with/except/import target; 0 = read).
//! - `kinds.bin`: per document, the named nodes' kind ids in preorder: u32 n, u64 offsets[n+1], u16 kind[].
//! - `kinds.txt`: kind names by kind id.
//! - `imports.bin`: resolved import edges between documents: u32 m, then m × (u32 from, u32 to).
//! - `summary.txt`: counts.
use std::collections::HashMap;
use std::io::{BufRead, Write};
use tree_sitter::{Node, Parser};

struct Doc { names: Vec<u32>, occ_name: Vec<u32>, occ_role: Vec<u8>, kinds: Vec<u16>, imports: Vec<(String, u32)> }

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str { std::str::from_utf8(&src[n.byte_range()]).unwrap_or("") }

/// Whether identifier `n` binds a name (see the module docs); `false` means a read.
fn is_binding(n: Node) -> bool {
    let Some(p) = n.parent() else { return false };
    let field = |q: Node, f: &str| q.child_by_field_name(f).map(|c| c.id() == n.id()).unwrap_or(false);
    match p.kind() {
        "function_definition" | "class_definition" => field(p, "name"),
        "parameters" | "lambda_parameters" | "list_splat_pattern" | "dictionary_splat_pattern" => true,
        "default_parameter" | "typed_default_parameter" => field(p, "name"),
        "typed_parameter" => true,
        "aliased_import" => field(p, "alias"),
        "as_pattern_target" => true,
        "global_statement" | "nonlocal_statement" => true,
        "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => field(p, "left"),
        "pattern_list" | "tuple_pattern" | "list_pattern" => {
            // a target list: binding when the list is itself an assignment/for target
            let mut q = p;
            while let Some(r) = q.parent() {
                match r.kind() {
                    "pattern_list" | "tuple_pattern" | "list_pattern" => q = r,
                    "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                        return r.child_by_field_name("left").map(|c| c.id() == q.id()).unwrap_or(false)
                    }
                    _ => return false,
                }
            }
            false
        }
        "dotted_name" => {
            // `import a.b` binds `a`; `from x import a` binds `a`
            let first = p.named_child(0).map(|c| c.id() == n.id()).unwrap_or(false);
            match p.parent().map(|g| g.kind()) {
                Some("import_statement") => first,
                Some("import_from_statement") => p.parent().unwrap().child_by_field_name("module_name").map(|m| m.id() != p.id()).unwrap_or(true),
                _ => false,
            }
        }
        _ => false,
    }
}

fn walk(n: Node, src: &[u8], vocab: &mut HashMap<String, u32>, kinds: &mut HashMap<&'static str, u16>, d: &mut Doc) {
    if n.is_named() {
        let k = n.kind();
        let next = kinds.len() as u16;
        d.kinds.push(*kinds.entry(k).or_insert(next));
    }
    match n.kind() {
        "identifier" => {
            // attribute names (`x.attr`) and keyword names (`f(k=1)`) are not variable references
            let skip = n.parent().map(|p| {
                (p.kind() == "attribute" && p.child_by_field_name("attribute").map(|c| c.id() == n.id()).unwrap_or(false))
                    || (p.kind() == "keyword_argument" && p.child_by_field_name("name").map(|c| c.id() == n.id()).unwrap_or(false))
            }).unwrap_or(false);
            if !skip {
                let t = text(n, src);
                let next = vocab.len() as u32;
                let id = *vocab.entry(t.to_owned()).or_insert(next);
                d.occ_name.push(id);
                d.occ_role.push(u8::from(is_binding(n)));
            }
        }
        "import_statement" => {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                let dn = if ch.kind() == "aliased_import" { ch.child_by_field_name("name") } else { Some(ch) };
                if let Some(dn) = dn { d.imports.push((text(dn, src).to_owned(), 0)); }
            }
        }
        "import_from_statement" => {
            if let Some(m) = n.child_by_field_name("module_name") {
                let module = text(m, src).replace(char::is_whitespace, "");
                let dots = module.chars().take_while(|&c| c == '.').count() as u32;
                let base = module.trim_start_matches('.').to_owned();
                let mut c = n.walk();
                let mut any = false;
                for ch in n.children_by_field_name("name", &mut c) {
                    let dn = if ch.kind() == "aliased_import" { ch.child_by_field_name("name") } else { Some(ch) };
                    if let Some(dn) = dn {
                        let sub = text(dn, src);
                        d.imports.push((if base.is_empty() { sub.to_owned() } else { format!("{base}.{sub}") }, dots));
                        any = true;
                    }
                }
                if !any || !base.is_empty() { d.imports.push((base, dots)); }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.children(&mut c) { walk(ch, src, vocab, kinds, d); }
}

fn csr<T: Copy>(out: &mut Vec<u8>, rows: &[&[T]], put: impl Fn(&mut Vec<u8>, T)) {
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    let mut off = 0u64;
    out.extend_from_slice(&off.to_le_bytes());
    for r in rows { off += r.len() as u64; out.extend_from_slice(&off.to_le_bytes()); }
    for r in rows { for &v in r.iter() { put(out, v); } }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (root, out) = (std::path::PathBuf::from(&args[1]), std::path::PathBuf::from(&args[2]));
    std::fs::create_dir_all(&out).unwrap();
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
    let mut vocab: HashMap<String, u32> = HashMap::new();
    let mut kinds: HashMap<&'static str, u16> = HashMap::new();
    let mut paths = Vec::new();
    let mut docs = Vec::new();
    let mut errors = 0;
    for line in std::io::stdin().lock().lines() {
        let path = std::path::PathBuf::from(line.unwrap());
        let Ok(src) = std::fs::read(&path) else { continue };
        let Some(tree) = parser.parse(&src, None) else { continue };
        if tree.root_node().has_error() { errors += 1; }
        let mut d = Doc { names: vec![], occ_name: vec![], occ_role: vec![], kinds: vec![], imports: vec![] };
        // tree-sitter-python's kind names live as long as the language: leak-free 'static via the grammar's table
        let root_node = tree.root_node();
        walk(root_node, &src, &mut vocab, &mut kinds, &mut d);
        let mut names = d.occ_name.clone();
        names.sort_unstable();
        names.dedup();
        d.names = names;
        paths.push(path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().into_owned());
        docs.push(d);
    }
    // module names: path without `.py`, `/` as `.`, `__init__` dropped
    let module_of = |p: &str| {
        let m = p.trim_end_matches(".py").replace('/', ".");
        m.strip_suffix(".__init__").map(str::to_owned).unwrap_or(m)
    };
    let modules: HashMap<String, u32> = paths.iter().enumerate().map(|(i, p)| (module_of(p), i as u32)).collect();
    let (mut edges, mut unresolved) = (Vec::new(), 0u64);
    for (i, d) in docs.iter().enumerate() {
        let here = module_of(&paths[i]);
        let package: Vec<&str> = if paths[i].ends_with("__init__.py") { here.split('.').collect() } else { here.split('.').collect::<Vec<_>>().split_last().map(|(_, r)| r.to_vec()).unwrap_or_default() };
        let mut seen = std::collections::HashSet::new();
        for (m, dots) in &d.imports {
            let full = if *dots == 0 { m.clone() } else {
                let keep = package.len().saturating_sub(*dots as usize - 1);
                let mut parts: Vec<&str> = package[..keep].to_vec();
                if !m.is_empty() { parts.push(m); }
                parts.join(".")
            };
            // the longest prefix of the dotted name that is a document
            let mut cand = full.as_str();
            let hit = loop {
                if let Some(&j) = modules.get(cand) { break Some(j); }
                match cand.rfind('.') { Some(k) => cand = &cand[..k], None => break None }
            };
            match hit { Some(j) if j as usize != i => { if seen.insert(j) { edges.push((i as u32, j)); } } Some(_) => {} None => unresolved += 1 }
        }
    }
    let mut vocab_list = vec![String::new(); vocab.len()];
    for (k, v) in &vocab { vocab_list[*v as usize] = k.clone(); }
    let mut kind_list = vec![""; kinds.len()];
    for (k, v) in &kinds { kind_list[*v as usize] = k; }
    std::fs::write(out.join("files.txt"), paths.join("\n") + "\n").unwrap();
    std::fs::write(out.join("names.txt"), vocab_list.join("\n") + "\n").unwrap();
    std::fs::write(out.join("kinds.txt"), kind_list.join("\n") + "\n").unwrap();
    let mut b = Vec::new();
    csr(&mut b, &docs.iter().map(|d| d.names.as_slice()).collect::<Vec<_>>(), |o, v| o.extend_from_slice(&v.to_le_bytes()));
    std::fs::write(out.join("doc_names.bin"), &b).unwrap();
    let mut b = Vec::new();
    csr(&mut b, &docs.iter().map(|d| d.occ_name.as_slice()).collect::<Vec<_>>(), |o, v| o.extend_from_slice(&v.to_le_bytes()));
    for d in &docs { b.extend_from_slice(&d.occ_role); }
    std::fs::write(out.join("occ.bin"), &b).unwrap();
    let mut b = Vec::new();
    csr(&mut b, &docs.iter().map(|d| d.kinds.as_slice()).collect::<Vec<_>>(), |o, v| o.extend_from_slice(&v.to_le_bytes()));
    std::fs::write(out.join("kinds.bin"), &b).unwrap();
    let mut b = (edges.len() as u32).to_le_bytes().to_vec();
    for (x, y) in &edges { b.extend_from_slice(&x.to_le_bytes()); b.extend_from_slice(&y.to_le_bytes()); }
    std::fs::write(out.join("imports.bin"), &b).unwrap();
    let occ: usize = docs.iter().map(|d| d.occ_name.len()).sum();
    let bind: usize = docs.iter().map(|d| d.occ_role.iter().filter(|&&r| r == 1).count()).sum();
    let pairs: usize = docs.iter().map(|d| d.names.len()).sum();
    let nodes: usize = docs.iter().map(|d| d.kinds.len()).sum();
    let mut s = std::fs::File::create(out.join("summary.txt")).unwrap();
    writeln!(s, "documents {}\nwith parse errors {errors}\nnamed nodes {nodes}\nkinds {}\nnames {}\nidentifier occurrences {occ} (bindings {bind})\n(document, name) pairs {pairs}\nimport edges {} (unresolved import names {unresolved})", docs.len(), kinds.len(), vocab.len(), edges.len()).unwrap();
}
