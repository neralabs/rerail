//! SQLite values, affinities, comparisons and scalar functions, as SQLite 3.53 (bundled by the
//! native engine) defines them for the constructs the batch-6 policies use.
use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum V {
    Null,
    Int(i64),
    Real(f64),
    Text(Rc<str>),
}

impl V {
    pub fn text(s: &str) -> V {
        V::Text(Rc::from(s))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, V::Null)
    }
    pub fn bool(b: bool) -> V {
        V::Int(b as i64)
    }
}

/// Column / expression affinity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aff {
    /// BLOB / none.
    None,
    Text,
    Numeric,
    Integer,
    Real,
}

impl Aff {
    pub fn numeric_class(self) -> bool {
        matches!(self, Aff::Numeric | Aff::Integer | Aff::Real)
    }
    /// The affinity of a declared type name (`CAST` targets, CTAS declared types).
    pub fn of_type(ty: &str) -> Aff {
        let t = ty.to_ascii_uppercase();
        if t.contains("INT") {
            Aff::Integer
        } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
            Aff::Text
        } else if t.contains("BLOB") || t.is_empty() {
            Aff::None
        } else if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
            Aff::Real
        } else {
            Aff::Numeric
        }
    }
}

/// SQLite's numeric reading of a text: `Some` when the whole text (surrounding spaces allowed) is a
/// well-formed integer or real literal.
fn text_to_number(s: &str) -> Option<V> {
    let t = s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\x0c' || c == '\x0b');
    if t.is_empty() {
        return None;
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        i = 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - digits_start;
    if i == bytes.len() && int_digits > 0 {
        if let Ok(v) = t.parse::<i64>() {
            return Some(V::Int(v));
        }
        return t.parse::<f64>().ok().map(V::Real);
    }
    // Real: digits [. digits] [e[+-]digits]
    let mut frac_digits = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let s2 = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - s2;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let s3 = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == s3 {
            return None;
        }
    }
    if i != bytes.len() {
        return None;
    }
    let f: f64 = t.parse().ok()?;
    // A real that is an exact integer stays REAL under NUMERIC affinity only if it has a decimal
    // point or exponent; SQLite converts "3.0" to INTEGER 3 under NUMERIC affinity.
    Some(V::Real(f))
}

/// Applies an affinity to a value (column storage, comparisons).
pub fn apply_affinity(v: V, aff: Aff) -> V {
    match aff {
        Aff::None => v,
        Aff::Text => match v {
            V::Int(i) => V::text(&i.to_string()),
            V::Real(f) => V::text(&real_to_text(f)),
            o => o,
        },
        Aff::Numeric | Aff::Integer => match &v {
            V::Text(s) => match text_to_number(s) {
                Some(V::Real(f)) if f.fract() == 0.0 && f.abs() < 9.2e18 => V::Int(f as i64),
                Some(n) => n,
                None => v,
            },
            V::Real(f) if *f == f.trunc() && f.abs() < 9.2e18 && aff == Aff::Integer => V::Int(*f as i64),
            V::Real(f) if *f == f.trunc() && f.abs() < 9.2e18 => V::Int(*f as i64),
            _ => v,
        },
        Aff::Real => match &v {
            V::Text(s) => match text_to_number(s) {
                Some(V::Int(i)) => V::Real(i as f64),
                Some(n) => n,
                None => v,
            },
            V::Int(i) => V::Real(*i as f64),
            _ => v,
        },
    }
}

/// The affinity to apply to both operands of a comparison (datatype3 §4.2).
pub fn comparison_affinity(a: Aff, b: Aff) -> (Aff, Aff) {
    if a.numeric_class() && !b.numeric_class() {
        return (Aff::None, Aff::Numeric);
    }
    if b.numeric_class() && !a.numeric_class() {
        return (Aff::Numeric, Aff::None);
    }
    if a == Aff::Text && b == Aff::None {
        return (Aff::None, Aff::Text);
    }
    if b == Aff::Text && a == Aff::None {
        return (Aff::Text, Aff::None);
    }
    (Aff::None, Aff::None)
}

/// Comparison affinity conversion: numeric affinity applied for a comparison converts a text only
/// when it is a well-formed number (and leaves an integral real alone).
pub fn compare_affinity(v: &V, aff: Aff) -> V {
    match aff {
        Aff::None => v.clone(),
        Aff::Text => apply_affinity(v.clone(), Aff::Text),
        _ => match v {
            V::Text(s) => text_to_number(s).unwrap_or_else(|| v.clone()),
            o => o.clone(),
        },
    }
}

fn class(v: &V) -> u8 {
    match v {
        V::Null => 0,
        V::Int(_) | V::Real(_) => 1,
        V::Text(_) => 2,
    }
}

/// Total order of SQLite values (BINARY collation): NULL < numbers < text.
pub fn cmp(a: &V, b: &V) -> Ordering {
    match (a, b) {
        (V::Int(x), V::Int(y)) => x.cmp(y),
        (V::Int(x), V::Real(y)) => cmp_int_real(*x, *y),
        (V::Real(x), V::Int(y)) => cmp_int_real(*y, *x).reverse(),
        (V::Real(x), V::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (V::Text(x), V::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        _ => class(a).cmp(&class(b)),
    }
}

fn cmp_int_real(i: i64, r: f64) -> Ordering {
    let f = i as f64;
    match f.partial_cmp(&r) {
        Some(Ordering::Equal) => {
            // Exact comparison for large values.
            if r >= 9.2e18 {
                Ordering::Less
            } else if r <= -9.2e18 {
                Ordering::Greater
            } else {
                i.cmp(&(r as i64)).then_with(|| {
                    if (r as i64) as f64 == r { Ordering::Equal } else if r > (r as i64) as f64 { Ordering::Less } else { Ordering::Greater }
                })
            }
        }
        Some(o) => o,
        None => Ordering::Less,
    }
}

/// `a OP b` with the comparison affinities already decided; `None` when either side is NULL.
pub fn compare(a: &V, b: &V, aff: (Aff, Aff)) -> Option<Ordering> {
    if a.is_null() || b.is_null() {
        return None;
    }
    let a = compare_affinity(a, aff.0);
    let b = compare_affinity(b, aff.1);
    Some(cmp(&a, &b))
}

/// A hashable, normalised key for equality under SQLite comparison (after affinity).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Int(i64),
    Real(u64),
    Text(Rc<str>),
}

pub fn key(v: &V) -> Option<Key> {
    match v {
        V::Null => None,
        V::Int(i) => Some(Key::Int(*i)),
        V::Real(f) => {
            if f.fract() == 0.0 && f.abs() < 9.2e18 {
                Some(Key::Int(*f as i64))
            } else {
                Some(Key::Real(f.to_bits()))
            }
        }
        V::Text(s) => Some(Key::Text(s.clone())),
    }
}

/// Key for DISTINCT / UNION / GROUP BY: NULLs are equal to each other there.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DKey {
    Null,
    K(Key),
}
pub fn dkey(v: &V) -> DKey {
    key(v).map_or(DKey::Null, DKey::K)
}

pub fn real_to_text(f: f64) -> String {
    // SQLite renders reals with "%!.15g".
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if f == f.trunc() && f.abs() < 1e15 {
        return format!("{:.1}", f);
    }
    let s = format!("{:.15e}", f);
    // Use shortest of %.15g semantics.
    let g = format_g15(f);
    let _ = s;
    g
}

fn format_g15(f: f64) -> String {
    let exp = f.abs().log10().floor() as i32;
    if !(-4..15).contains(&exp) {
        let s = format!("{:.14e}", f);
        let (m, e) = s.split_once('e').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap();
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (14 - exp).max(0) as usize;
        let s = format!("{:.*}", decimals, f);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        if s.contains('.') { s } else { format!("{s}.0") }
    }
}

pub fn to_text(v: &V) -> Option<Rc<str>> {
    match v {
        V::Null => None,
        V::Int(i) => Some(Rc::from(i.to_string().as_str())),
        V::Real(f) => Some(Rc::from(real_to_text(*f).as_str())),
        V::Text(s) => Some(s.clone()),
    }
}

/// Numeric value of an operand of arithmetic (text read by its longest numeric prefix).
pub fn to_numeric(v: &V) -> Option<V> {
    match v {
        V::Null => None,
        V::Int(_) | V::Real(_) => Some(v.clone()),
        V::Text(s) => Some(numeric_prefix(s)),
    }
}

/// SQLite's `sqlite3AtoF`-style longest numeric prefix (leading spaces skipped); 0 when none.
pub fn numeric_prefix(s: &str) -> V {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0c', '\x0b']);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_end = i;
    let mut is_real = false;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 || int_end > ds {
            is_real = j > i + 1 || true;
            i = j;
        }
    }
    if int_end == ds && !is_real {
        return V::Int(0);
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            is_real = true;
            i = j;
        }
    }
    let p = &t[..i];
    if !is_real {
        match p.parse::<i64>() {
            Ok(v) => V::Int(v),
            Err(_) => V::Real(p.parse::<f64>().unwrap_or(0.0)),
        }
    } else {
        V::Real(p.parse::<f64>().unwrap_or(0.0))
    }
}

/// Truth of a value in a boolean context; `None` for NULL.
pub fn truth(v: &V) -> Option<bool> {
    match v {
        V::Null => None,
        V::Int(i) => Some(*i != 0),
        V::Real(f) => Some(*f != 0.0),
        V::Text(s) => match numeric_prefix(s) {
            V::Int(i) => Some(i != 0),
            V::Real(f) => Some(f != 0.0),
            _ => Some(false),
        },
    }
}

pub fn cast(v: &V, aff: Aff) -> V {
    match (v, aff) {
        (V::Null, _) => V::Null,
        (_, Aff::Text) => to_text(v).map_or(V::Null, V::Text),
        (V::Text(s), Aff::Integer) => match numeric_prefix(s) {
            V::Int(i) => V::Int(i),
            V::Real(f) => V::Int(f as i64),
            o => o,
        },
        (V::Real(f), Aff::Integer) => V::Int(*f as i64),
        (V::Int(_), Aff::Integer) => v.clone(),
        (V::Text(s), Aff::Real) => match numeric_prefix(s) {
            V::Int(i) => V::Real(i as f64),
            o => o,
        },
        (V::Int(i), Aff::Real) => V::Real(*i as f64),
        (V::Real(_), Aff::Real) => v.clone(),
        (V::Text(s), Aff::Numeric) => match numeric_prefix(s) {
            V::Real(f) if f == f.trunc() && f.abs() < 9.2e18 => V::Int(f as i64),
            o => o,
        },
        (V::Real(f), Aff::Numeric) if *f == f.trunc() && f.abs() < 9.2e18 => V::Int(*f as i64),
        (_, Aff::Numeric) => v.clone(),
        (_, Aff::None) => v.clone(),
    }
}

pub fn arith(op: char, a: &V, b: &V) -> V {
    let (Some(a), Some(b)) = (to_numeric(a), to_numeric(b)) else { return V::Null };
    match (&a, &b) {
        (V::Int(x), V::Int(y)) => {
            let r = match op {
                '+' => x.checked_add(*y),
                '-' => x.checked_sub(*y),
                '*' => x.checked_mul(*y),
                '/' => {
                    if *y == 0 {
                        return V::Null;
                    }
                    x.checked_div(*y)
                }
                '%' => {
                    if *y == 0 {
                        return V::Null;
                    }
                    x.checked_rem(*y)
                }
                _ => None,
            };
            match r {
                Some(v) => V::Int(v),
                None => {
                    let (fx, fy) = (*x as f64, *y as f64);
                    V::Real(match op {
                        '+' => fx + fy,
                        '-' => fx - fy,
                        '*' => fx * fy,
                        '/' => fx / fy,
                        _ => return V::Null,
                    })
                }
            }
        }
        _ => {
            let fx = match a { V::Int(i) => i as f64, V::Real(f) => f, _ => 0.0 };
            let fy = match b { V::Int(i) => i as f64, V::Real(f) => f, _ => 0.0 };
            match op {
                '+' => V::Real(fx + fy),
                '-' => V::Real(fx - fy),
                '*' => V::Real(fx * fy),
                '/' => {
                    if fy == 0.0 {
                        V::Null
                    } else {
                        V::Real(fx / fy)
                    }
                }
                '%' => {
                    let (ix, iy) = (fx as i64, fy as i64);
                    if iy == 0 {
                        V::Null
                    } else {
                        V::Real((ix % iy) as f64)
                    }
                }
                _ => V::Null,
            }
        }
    }
}

/// `LIKE` with SQLite's default: ASCII case-insensitive, `%` and `_`, no escape.
pub fn like(pattern: &str, text: &str) -> bool {
    fn lower(c: char) -> char {
        c.to_ascii_lowercase()
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        let mut pi = 0;
        let mut ti = 0;
        let mut star: Option<(usize, usize)> = None;
        while ti < t.len() {
            if pi < p.len() && p[pi] == '%' {
                star = Some((pi, ti));
                pi += 1;
            } else if pi < p.len() && (p[pi] == '_' || lower(p[pi]) == lower(t[ti])) {
                pi += 1;
                ti += 1;
            } else if let Some((sp, st)) = star {
                pi = sp + 1;
                ti = st + 1;
                star = Some((sp, st + 1));
            } else {
                return false;
            }
        }
        while pi < p.len() && p[pi] == '%' {
            pi += 1;
        }
        pi == p.len()
    }
    go(&p, &t)
}

pub fn substr(s: &str, start: i64, len: Option<i64>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let mut p1 = start;
    let (mut p2, neg) = match len {
        None => (i64::MAX / 4, false),
        Some(l) if l < 0 => (-l, true),
        Some(l) => (l, false),
    };
    if p1 < 0 {
        p1 += n;
        if p1 < 0 {
            p2 += p1;
            if p2 < 0 {
                p2 = 0;
            }
            p1 = 0;
        }
    } else if p1 > 0 {
        p1 -= 1;
    } else if p2 > 0 {
        p2 -= 1;
    }
    if neg {
        p1 -= p2;
        if p1 < 0 {
            p2 += p1;
            p1 = 0;
        }
    }
    if p1 + p2 > n {
        p2 = n - p1;
        if p2 < 0 {
            p2 = 0;
        }
    }
    if p1 >= n || p2 <= 0 {
        return String::new();
    }
    chars[p1 as usize..(p1 + p2) as usize].iter().collect()
}

pub fn trim(s: &str, chars: Option<&str>, left: bool, right: bool) -> String {
    let set: Vec<char> = chars.map_or_else(|| vec![' '], |c| c.chars().collect());
    let mut v: &str = s;
    if left {
        v = v.trim_start_matches(|c| set.contains(&c));
    }
    if right {
        v = v.trim_end_matches(|c| set.contains(&c));
    }
    v.to_owned()
}

pub fn json_value_to_sql(j: &serde_json::Value) -> V {
    match j {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Bool(b) => V::Int(*b as i64),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                V::Int(i)
            } else {
                V::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => V::text(s),
        other => V::text(&serde_json::to_string(other).unwrap()),
    }
}

/// `json_extract(json, path)` for paths `$`, `$[N]`, `$.key`, `$."key"` chains.
pub fn json_extract(json: &str, path: &str) -> Result<V, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("malformed JSON: {e}"))?;
    let mut cur = &v;
    let p = path.strip_prefix('$').ok_or("bad JSON path")?;
    let mut rest = p;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('[') {
            let close = r.find(']').ok_or("bad JSON path")?;
            let idx: usize = r[..close].trim().parse().map_err(|_| "bad JSON path")?;
            match cur.get(idx) {
                Some(x) => cur = x,
                None => return Ok(V::Null),
            }
            rest = &r[close + 1..];
        } else if let Some(r) = rest.strip_prefix('.') {
            let (key, after) = if let Some(q) = r.strip_prefix('"') {
                let close = q.find('"').ok_or("bad JSON path")?;
                (&q[..close], &q[close + 1..])
            } else {
                let end = r.find(['.', '[']).unwrap_or(r.len());
                (&r[..end], &r[end..])
            };
            match cur.get(key) {
                Some(x) => cur = x,
                None => return Ok(V::Null),
            }
            rest = after;
        } else {
            return Err("bad JSON path".into());
        }
    }
    Ok(json_value_to_sql(cur))
}

pub fn json_quote(v: &V) -> V {
    match v {
        V::Null => V::text("null"),
        V::Int(i) => V::text(&i.to_string()),
        V::Real(f) => V::text(&real_to_text(*f)),
        V::Text(s) => V::text(&serde_json::to_string(&**s).unwrap()),
    }
}
