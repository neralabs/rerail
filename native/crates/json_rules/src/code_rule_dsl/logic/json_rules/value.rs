//! JSON as Python's `json` module reads and writes it: objects keep their keys in file order (a repeated key keeps
//! its first place and its last value), numbers are ints or floats, and `dumps` writes what `json.dumps` writes
//! (floats as Python's `repr`, `ensure_ascii` escaping, the separators asked for).

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Obj(Obj),
}

/// A JSON object: its entries in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Obj(pub Vec<(String, Value)>);

impl Obj {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.iter().find(|(key, _)| key == k).map(|(_, v)| v)
    }
    pub fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(key, _)| key == k)
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
    /// `set(d) == {keys…}`.
    pub fn key_set_is(&self, keys: &[&str]) -> bool {
        self.0.len() == keys.len() && keys.iter().all(|k| self.has(k))
    }
    /// `set(d) <= {keys…}`.
    pub fn keys_within(&self, keys: &[&str]) -> bool {
        self.0.iter().all(|(k, _)| keys.contains(&k.as_str()))
    }
    /// The entries whose key is not in `drop`, in order.
    pub fn without(&self, drop: &[&str]) -> Obj {
        Obj(self.0.iter().filter(|(k, _)| !drop.contains(&k.as_str())).cloned().collect())
    }
    pub fn insert(&mut self, k: &str, v: Value) {
        if let Some(slot) = self.0.iter_mut().find(|(key, _)| key == k) {
            slot.1 = v;
        } else {
            self.0.push((k.to_owned(), v));
        }
    }
}

impl Value {
    pub fn obj(entries: Vec<(&str, Value)>) -> Value {
        Value::Obj(Obj(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()))
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_obj(&self) -> Option<&Obj> {
        if let Value::Obj(o) = self { Some(o) } else { None }
    }
    pub fn as_list(&self) -> Option<&Vec<Value>> {
        if let Value::List(l) = self { Some(l) } else { None }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }
    pub fn is_obj(&self) -> bool {
        matches!(self, Value::Obj(_))
    }
    pub fn is_list(&self) -> bool {
        matches!(self, Value::List(_))
    }
    /// `isinstance(v, int)`: a bool is an int in Python.
    pub fn is_int(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Bool(_))
    }
    /// `isinstance(v, int) and not isinstance(v, bool)`.
    pub fn is_plain_int(&self) -> bool {
        matches!(self, Value::Int(_))
    }
    /// `isinstance(v, (int, float))`, bools included.
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Bool(_) | Value::Float(_))
    }
    /// Python's int value of an int (or bool).
    pub fn int(&self) -> Option<i128> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i128),
            _ => None,
        }
    }
    pub fn is_true(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    pub fn is_false(&self) -> bool {
        matches!(self, Value::Bool(false))
    }
    /// Python's truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Obj(o) => !o.is_empty(),
        }
    }
    /// `d.get(k)` of a dict value (None for any other value or a missing key).
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.as_obj().and_then(|o| o.get(k))
    }
    pub fn has(&self, k: &str) -> bool {
        self.as_obj().is_some_and(|o| o.has(k))
    }
}

/// `opt.truthy()` of a `d.get(k)`.
pub fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(Value::truthy)
}

// ---- reading --------------------------------------------------------------------------------------------------

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, text };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("extra data at byte {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    text: &'a str,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.i))
    }
    fn value(&mut self) -> Result<Value, String> {
        match self.s.get(self.i) {
            None => self.err("expecting value"),
            Some(b'{') => {
                self.i += 1;
                let mut obj = Obj::default();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(obj));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("expecting property name");
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expecting ':'");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    obj.insert(&k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(obj));
                        }
                        _ => return self.err("expecting ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut list = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::List(list));
                }
                loop {
                    self.ws();
                    list.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::List(list));
                        }
                        _ => return self.err("expecting ',' or ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(_) => {
                let rest = &self.text[self.i..];
                for (word, v) in [("true", Value::Bool(true)), ("false", Value::Bool(false)), ("null", Value::Null)] {
                    if rest.starts_with(word) {
                        self.i += word.len();
                        return Ok(v);
                    }
                }
                self.number()
            }
        }
    }
    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        let s = self.s;
        let mut j = self.i;
        if s.get(j) == Some(&b'-') {
            j += 1;
        }
        let digits = |j: &mut usize| {
            let k = *j;
            while *j < s.len() && s[*j].is_ascii_digit() {
                *j += 1;
            }
            *j > k
        };
        if !digits(&mut j) {
            return self.err("expecting value");
        }
        let mut float = false;
        if s.get(j) == Some(&b'.') && s.get(j + 1).is_some_and(u8::is_ascii_digit) {
            j += 1;
            digits(&mut j);
            float = true;
        }
        if matches!(s.get(j), Some(b'e' | b'E')) {
            let mut k = j + 1;
            if matches!(s.get(k), Some(b'+' | b'-')) {
                k += 1;
            }
            if digits(&mut k) {
                j = k;
                float = true;
            }
        }
        self.i = j;
        let t = &self.text[start..j];
        if float {
            t.parse::<f64>().map(Value::Float).map_err(|e| e.to_string())
        } else {
            t.parse::<i128>().map(Value::Int).map_err(|e| e.to_string())
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return self.err("unterminated string") };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let Some(&e) = self.s.get(self.i + 1) else { return self.err("unterminated string") };
                    self.i += 2;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) && self.text[self.i..].starts_with("\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    self.i = save;
                                    hi
                                }
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return self.err("invalid escape"),
                    }
                }
                _ => {
                    let ch = self.text[self.i..].chars().next().expect("a char");
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.text.get(self.i..self.i + 4).ok_or("bad \\u escape")?;
        self.i += 4;
        u32::from_str_radix(h, 16).map_err(|e| e.to_string())
    }
}

// ---- writing --------------------------------------------------------------------------------------------------

/// `json.dumps(s, ensure_ascii=…)` of a string.
pub fn dump_str(s: &str, ascii: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if ascii && (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{:04x}", u);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Python's `repr(float)` (and `str(float)`): the shortest text that reads back, `1e+16` past 16 digits.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Rust's `{:e}` is the shortest round-trip digits: d[.ddd]e<exp>.
    let e = format!("{:e}", f);
    let (mant, exp) = e.split_once('e').expect("an exponent");
    let exp: i32 = exp.parse().expect("an exponent");
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let sign = if neg { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        let body = if exp >= 0 {
            if n > exp + 1 {
                format!("{}.{}", &digits[..(exp + 1) as usize], &digits[(exp + 1) as usize..])
            } else {
                format!("{}{}.0", digits, "0".repeat((exp + 1 - n) as usize))
            }
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{body}")
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        format!("{sign}{m}e{}{:02}", if exp < 0 { "-" } else { "+" }, exp.abs())
    }
}

/// `json.dumps(v, ensure_ascii=ascii, separators=(item, key), sort_keys=sort)`.
pub fn dumps(v: &Value, ascii: bool, item: &str, key: &str, sort: bool) -> String {
    let mut out = String::new();
    dump_into(&mut out, v, ascii, item, key, sort);
    out
}

fn dump_into(out: &mut String, v: &Value, ascii: bool, item: &str, key: &str, sort: bool) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(f) => out.push_str(&float_repr(*f)),
        Value::Str(s) => out.push_str(&dump_str(s, ascii)),
        Value::List(l) => {
            out.push('[');
            for (k, x) in l.iter().enumerate() {
                if k > 0 {
                    out.push_str(item);
                }
                dump_into(out, x, ascii, item, key, sort);
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            let mut entries: Vec<&(String, Value)> = o.0.iter().collect();
            if sort {
                entries.sort_by(|a, b| a.0.cmp(&b.0));
            }
            for (k, (name, x)) in entries.into_iter().enumerate() {
                if k > 0 {
                    out.push_str(item);
                }
                out.push_str(&dump_str(name, ascii));
                out.push_str(key);
                dump_into(out, x, ascii, item, key, sort);
            }
            out.push('}');
        }
    }
}

/// `json.dumps(v, ensure_ascii=False)`: the default separators.
pub fn q_val(v: &Value) -> String {
    dumps(v, false, ", ", ": ", false)
}

/// `json.dumps(s, ensure_ascii=False)` of a string.
pub fn q(s: &str) -> String {
    dump_str(s, false)
}

/// `json.dumps(obj, ensure_ascii=False, separators=(", ", ": "))`: the translator's `compact`.
pub fn compact(v: &Value) -> String {
    dumps(v, false, ", ", ": ", false)
}

/// Python's `str(v)` as an f-string prints a JSON value: ints, floats, `True`, strings bare.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => float_repr(*f),
        Value::Str(s) => s.clone(),
        Value::List(l) => format!("[{}]", l.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Obj(o) => format!("{{{}}}", o.0.iter().map(|(k, v)| format!("{}: {}", py_repr_str(k), py_repr(v))).collect::<Vec<_>>().join(", ")),
    }
}

/// Python's `repr(v)`.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::Str(s) => py_repr_str(s),
        other => py_str(other),
    }
}

/// Python's `repr(str)`: single quotes unless the text holds one and no double quote.
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

