//! Native grammar validation for Python AST consumers. Raw source remains available on a rejected parse.
//! The tape owns the demanded, cached verdict; no Python interpreter is linked or invoked.
use ruff_python_ast::PythonVersion;
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use crate::code_rule_dsl::facts::core::PythonSyntaxError;
use ruff_python_ast::{Expr, token::{TokenKind,Tokens}, visitor::{self,Visitor}};
use ruff_text_size::Ranged;
mod diagnostic;

/// CPython compares indentation using both eight-column tabs and one-column tabs.
/// The native parser supplies logical newlines, so strings and continued groups never enter this stack.
fn tab_error(source:&str,tokens:&Tokens) -> Option<(u32,PythonSyntaxError)> {
    let mut stack=vec![(0usize,0usize)];let mut head=true;
    for token in tokens.iter() {
        let (kind,range)=token.as_tuple();
        if kind==TokenKind::Newline {head=true;continue;}
        if matches!(kind,TokenKind::NonLogicalNewline|TokenKind::Comment|TokenKind::Indent|TokenKind::Dedent|TokenKind::EndOfFile) {continue;}
        if !head {continue;} head=false;
        let offset=u32::from(range.start()) as usize;let start=source[..offset].rfind('\n').map_or(0,|i|i+1);
        let (mut col,mut alt)=(0,0);
        for c in source[start..offset].chars() {
            match c {' '=>{col+=1;alt+=1;},'\t'=>{col=(col/8+1)*8;alt+=1;},'\x0c'=>{col=0;alt=0;},_=>break}
        }
        while stack.len()>1 && col<stack.last().unwrap().0 {stack.pop();}
        let previous=*stack.last().unwrap();
        if col==previous.0 && alt!=previous.1 || col>previous.0 && alt<=previous.1 {
            let row=source[..start].bytes().filter(|&c|c==b'\n').count() as i64+1;
            let stop=source[start..].find('\n').map_or(source.len(),|i|start+i+1);
            return Some((offset as u32,PythonSyntaxError {kind:"TabError".into(),message:"inconsistent use of tabs and spaces in indentation".into(),
                line:Some(row),offset:Some(1),end_line:Some(row),end_offset:Some(0),text:Some(source[start..stop].into())}));
        }
        if col>previous.0 {stack.push((col,alt));}
    }
    None
}

struct Before { limit:u32, best:Option<(u32,u32)> }
impl<'a> Visitor<'a> for Before {
    fn visit_expr(&mut self,expr:&'a Expr) {
        let (start,end)=(u32::from(expr.start()),u32::from(expr.end()));
        if end<=self.limit && self.best.is_none_or(|(a,b)|end>b || end==b && start<a) {self.best=Some((start,end));}
        visitor::walk_expr(self,expr);
    }
}

pub(super) fn error(source: &str) -> Option<(u32, String)> {
    exception(source).map(|e| e.verdict())
}

pub(super) fn exception(source: &str) -> Option<PythonSyntaxError> {
    if source.contains('\0') {
        return Some(PythonSyntaxError { kind:"SyntaxError".into(), message:"source code string cannot contain null bytes".into(),
            line:None, offset:None, end_line:None, end_offset:None, text:None });
    }
    let mut normalized=source.replace("\r\n","\n").replace('\r',"\n");
    if !normalized.ends_with('\n') {normalized.push('\n');}
    let source=normalized.as_str();
    // ast.parse(str) does not apply the file decoder's UTF-8 signature removal.
    if source.starts_with('\u{feff}') {
        return Some(PythonSyntaxError { kind:"SyntaxError".into(), message:"invalid non-printable character U+FEFF".into(),
            line:Some(1), offset:Some(1), end_line:Some(1), end_offset:Some(1), text:Some(source.lines().next().unwrap_or("").into()) });
    }
    let parsed = parse_unchecked(source, ParseOptions::from(Mode::Module).with_target_version(PythonVersion::PY313));
    // Choose the parser error before adjusting its displayed position. An unclosed group can
    // point back to its opener, but must not hide an earlier malformed parameter.
    let ordinary = parsed.errors().iter().min_by_key(|e| e.location.start());
    let version = parsed.unsupported_syntax_errors().iter().min_by_key(|e| e.range.start());
    if let Some((at,error))=tab_error(source,parsed.tokens()) {
        let first=ordinary.map(|e|u32::from(e.location.start())).into_iter().chain(version.map(|e|u32::from(e.range.start()))).min();
        // The parser can report the indentation range before its first token.
        // A tab inconsistency on that same logical line precedes that error.
        let line_start=source[..at as usize].rfind('\n').map_or(0,|i|i+1) as u32;
        if first.is_none_or(|first|line_start<=first) {return Some(error);}
    }
    let (mut offset, message, mut end) = match (ordinary, version) {
        (Some(e), v) if v.is_none_or(|v| e.location.start() <= v.range.start()) =>
            { let (at,message)=diagnostic::describe(source,e,parsed.tokens()); (at,message,u32::from(e.location.end())) },
        (_, Some(e)) => (u32::from(e.range.start()), e.to_string(), u32::from(e.range.end())),
        _ => return None,
    };
    let indent=message.starts_with("expected an indented block") || message=="unexpected indent" || message.starts_with("unindent does not match");
    let kind=if message.contains("inconsistent use of tabs") {"TabError"} else if indent {"IndentationError"} else {"SyntaxError"};
    let eof_indent=indent && source[(offset as usize).min(source.len())..].trim().is_empty();
    if eof_indent && source.ends_with('\n') {offset=source.len().saturating_sub(1) as u32;end=offset;}
    let position=|at:u32| {
        let at=(at as usize).min(source.len());let start=source[..at].rfind('\n').map_or(0,|i|i+1);
        let row=source[..start].bytes().filter(|&c|c==b'\n').count() as i64+1;
        (row,source[start..at].chars().count() as i64+1,start)
    };
    let (row,mut col,start)=position(offset);let (mut end_row,mut end_col,_)=position(end);
    let stop=source[start..].find('\n').map_or(source.len(),|i|start+i+1);
    let mut text=source[start..stop].to_owned();
    if end_row==row+1 && end>offset && source.as_bytes().get(end as usize-1)==Some(&b'\n') {
        end_row=row;end_col=text.chars().count() as i64+1;
    }
    if message.ends_with("was never closed") {
        end_row=row;end_col=0;
        if source[start..].lines().count()>1 {text=text.trim_end_matches('\n').into();}
    }
    else if message.starts_with("unmatched ") || message.starts_with("closing parenthesis ") {
        end_row=row;end_col=col;text=text.trim_end_matches('\n').into();
    } else if message.starts_with("unterminated ") {end_row=row;end_col=col;text=text.trim_end_matches('\n').into();}
    else if message.starts_with("f-string: expecting") {end_row=row;end_col=col;text=text.trim_end_matches('\n').into();}
    else if message=="unexpected character after line continuation character" {col+=1;end_row=row;end_col=0;}
    if message=="invalid syntax. Perhaps you forgot a comma?" {
        let mut before=Before{limit:offset,best:None};
        if let Some(module)=parsed.syntax().as_module() {before.visit_body(module.body.as_slice());}
        if let Some((start,_))=before.best {col=position(start).1;}
    }
    if message=="unexpected indent" {col=(end as usize).saturating_sub(start) as i64;end_row=row;end_col=-1;}
    if message.starts_with("unindent does not match") {col=text.chars().count() as i64;end_row=row;end_col=-1;}
    if eof_indent {end_row=row;end_col=-1;}
    Some(PythonSyntaxError {kind:kind.into(),message,line:Some(row),offset:Some(col),end_line:Some(end_row),end_offset:Some(end_col),text:Some(text)})
}

#[cfg(test)]
mod tests {
    #[test]
    fn complete_native_syntax_exceptions_match_python_313() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("python_syntax/python-exceptions.json")).unwrap();
        let mut failures=Vec::new();
        for row in cases.as_array().unwrap() {
            let actual = super::exception(row["source"].as_str().unwrap());
            let actual=serde_json::to_value(actual).unwrap();
            if actual!=row["exception"] { failures.push(serde_json::json!({"source":row["source"],"expected":row["exception"],"actual":actual})); }
        }
        assert!(failures.is_empty(),"{}",serde_json::to_string_pretty(&failures).unwrap());
    }

    #[test]
    fn malformed_parameter_list_has_the_python_syntax_message() {
        assert_eq!(super::error("def broken(:\n"), Some((1, "invalid syntax".to_owned())));
    }

    #[test]
    fn native_diagnostics_match_recorded_python_313_errors() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("python_syntax/python-errors.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            // NUL has no SyntaxError location; the relation uses the document's first line.
            let expected = (case["line"].as_u64().unwrap_or(1) as u32, case["message"].as_str().unwrap().to_owned());
            assert_eq!(super::error(source), Some(expected), "{source:?}");
        }
    }
}
