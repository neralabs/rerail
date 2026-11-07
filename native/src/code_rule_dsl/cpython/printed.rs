//! W's pythonAst fields that print a node, per located node of a file's CPython tree (`ast::module`): `unparse`
//! (`ast.unparse`, `unparse`), `dump` (`ast.dump(node, include_attributes=False)`, `dump`), and the hashes of dumps
//! W's worker gave: `dump_sha256`, and a def's `body_sha256` and `signature_sha256`. W's worker found a node by its
//! span and the classes its tree-sitter kind may be (`of_kind`, its `KINDS`).

use super::ast::{Node, module};
use super::dump::Dump;
use super::unparse::{is_expr, unparse};

/// The fields wanted.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Want {
    pub(crate) unparse: bool,
    pub(crate) dump: bool,
    pub(crate) dump_sha256: bool,
    pub(crate) body_sha256: bool,
    pub(crate) signature_sha256: bool,
}

impl Want {
    fn any(self) -> bool {
        self.unparse || self.dump || self.dump_sha256 || self.body_sha256 || self.signature_sha256
    }
}

/// One located node: its UTF-8 byte range, its class, and the fields wanted that it has.
#[derive(Clone, Debug, Default)]
pub(crate) struct Printed {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) class: &'static str,
    pub(crate) unparse: Option<String>,
    pub(crate) dump: Option<String>,
    pub(crate) dump_sha256: Option<String>,
    pub(crate) body_sha256: Option<String>,
    pub(crate) signature_sha256: Option<String>,
}

/// Each located node of `source`'s tree, in preorder (outer before inner), but an f-string's own pieces (its
/// `JoinedStr`'s values and their format specs: no tree-sitter kind is read as one), with the fields of `want` it
/// has. None where `ast.parse` rejects the file.
pub(crate) fn printed(source: &str, want: Want) -> Vec<Printed> {
    if !want.any() {
        return Vec::new();
    }
    let Some(root) = module(source) else { return Vec::new() };
    let dump = (want.dump || want.dump_sha256 || want.body_sha256 || want.signature_sha256).then(|| Dump::of(&root));
    let mut located = Vec::new();
    walk(&root, &mut located);
    located
        .into_iter()
        .filter_map(|n| {
            let (start, end) = n.span?;
            let def = matches!(n.class, "FunctionDef" | "AsyncFunctionDef");
            let d = dump.as_ref();
            Some(Printed {
                start,
                end,
                class: n.class,
                unparse: if want.unparse { unparse(n) } else { None },
                dump: if want.dump { d.and_then(|d| d.get(n)).map(str::to_owned) } else { None },
                dump_sha256: if want.dump_sha256 { d.and_then(|d| d.sha256(n)) } else { None },
                body_sha256: if want.body_sha256 && def { d.and_then(|d| d.body_sha256(n)) } else { None },
                signature_sha256: if want.signature_sha256 && def { d.and_then(|d| d.signature_sha256(n)) } else { None },
            })
        })
        .collect()
}

/// The nodes of `n`'s tree in preorder, an f-string's pieces left out (its fields' values are not).
fn walk<'n>(n: &'n Node, out: &mut Vec<&'n Node>) {
    use super::ast::F;
    fn field<'n>(f: &'n F, out: &mut Vec<&'n Node>) {
        match f {
            F::Node(n) => walk(n, out),
            F::List(items) => items.iter().for_each(|i| field(i, out)),
            _ => {}
        }
    }
    fn pieces<'n>(joined: &'n Node, out: &mut Vec<&'n Node>) {
        for v in joined.list("values") {
            let F::Node(v) = v else { continue };
            if v.class == "FormattedValue" {
                if let Some(value) = v.node("value") {
                    walk(value, out);
                }
                if let Some(spec) = v.node("format_spec") {
                    pieces(spec, out);
                }
            }
        }
    }
    out.push(n);
    if n.class == "JoinedStr" {
        pieces(n, out);
        return;
    }
    n.fields.iter().for_each(|(_, f)| field(f, out));
}

/// Whether W's worker reads a node of tree-sitter kind `kind` as one of CPython class `class` (its `KINDS`: `type`
/// is any expression).
pub(crate) fn of_kind(class: &str, kind: &str) -> bool {
    let classes: &[&str] = match kind {
        "type" => return is_expr(class),
        "function_definition" => &["FunctionDef", "AsyncFunctionDef"],
        "class_definition" => &["ClassDef"],
        "assignment" => &["Assign", "AnnAssign"],
        "identifier" => &["Name"],
        "attribute" => &["Attribute"],
        "call" => &["Call"],
        "string" | "concatenated_string" => &["Constant", "JoinedStr"],
        "integer" | "float" | "true" | "false" | "none" | "ellipsis" => &["Constant"],
        "tuple" | "tuple_pattern" | "pattern_list" => &["Tuple"],
        "list" | "list_pattern" => &["List"],
        "dictionary" => &["Dict"],
        "subscript" | "generic_type" => &["Subscript"],
        "import_from_statement" | "future_import_statement" => &["ImportFrom"],
        "import_statement" => &["Import"],
        "aliased_import" | "dotted_name" => &["alias"],
        "expression_statement" => &["Expr"],
        "augmented_assignment" => &["AugAssign"],
        "return_statement" => &["Return"],
        "if_statement" | "elif_clause" => &["If"],
        "for_statement" => &["For", "AsyncFor"],
        "while_statement" => &["While"],
        "with_statement" => &["With", "AsyncWith"],
        "try_statement" => &["Try", "TryStar"],
        "except_clause" => &["ExceptHandler"],
        "raise_statement" => &["Raise"],
        "assert_statement" => &["Assert"],
        "pass_statement" => &["Pass"],
        "break_statement" => &["Break"],
        "continue_statement" => &["Continue"],
        "delete_statement" => &["Delete"],
        "keyword_argument" | "dictionary_splat" => &["keyword"],
        "lambda" => &["Lambda"],
        "await" => &["Await"],
        "boolean_operator" => &["BoolOp"],
        "binary_operator" | "union_type" => &["BinOp"],
        "not_operator" | "unary_operator" => &["UnaryOp"],
        "comparison_operator" => &["Compare"],
        "conditional_expression" => &["IfExp"],
        "list_comprehension" => &["ListComp"],
        "dictionary_comprehension" => &["DictComp"],
        "set_comprehension" => &["SetComp"],
        "generator_expression" => &["GeneratorExp"],
        "named_expression" => &["NamedExpr"],
        "list_splat" => &["Starred"],
        "set" => &["Set"],
        "default_parameter" | "typed_parameter" | "typed_default_parameter" => &["arg"],
        _ => &[],
    };
    classes.contains(&class)
}

/// Whether W's worker, finding no node of kind `kind` at its span, takes the one node of its classes that starts
/// where it does and ends within it (a def, a class or an assignment whose tree-sitter node runs past CPython's).
pub(crate) fn by_start(kind: &str) -> bool {
    matches!(kind, "function_definition" | "class_definition" | "assignment")
}
