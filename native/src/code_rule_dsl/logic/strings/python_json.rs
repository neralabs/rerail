//! Python 3.13 JSON syntax diagnostics. This validates text; file decoding and
//! shared JSON storage remain the policy input reader's responsibility.
type Error = (&'static str, usize);
struct Parser { chars: Vec<char>, pos: usize }
impl Parser {
    fn peek(&self) -> Option<char> { self.chars.get(self.pos).copied() }
    fn space(&mut self) { while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) { self.pos += 1; } }
    fn literal(&mut self, s: &str) -> bool {
        let n = s.chars().count();
        if self.chars[self.pos..].iter().copied().take(n).eq(s.chars()) { self.pos += n; true } else { false }
    }
    fn string(&mut self) -> Result<(), Error> {
        let start = self.pos;
        self.pos += 1;
        while let Some(c) = self.peek() {
            match c {
                '"' => { self.pos += 1; return Ok(()); }
                '\\' => {
                    let slash = self.pos;
                    self.pos += 1;
                    match self.peek() {
                        Some('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') => self.pos += 1,
                        Some('u') => {
                            let u = self.pos;
                            self.pos += 1;
                            for _ in 0..4 {
                                if !self.peek().is_some_and(|c| c.is_ascii_hexdigit()) { return Err(("Invalid \\uXXXX escape",u)); }
                                self.pos += 1;
                            }
                        }
                        None => return Err(("Unterminated string starting at",start)),
                        _ => return Err(("Invalid \\escape",slash)),
                    }
                }
                c if c < ' ' => return Err(("Invalid control character at",self.pos)),
                _ => self.pos += 1,
            }
        }
        Err(("Unterminated string starting at",start))
    }
    fn value(&mut self) -> Result<(), Error> {
        self.space();
        match self.peek() {
            Some('"') => self.string(),
            Some('{') => self.container(true),
            Some('[') => self.container(false),
            _ => {
                for word in ["null","true","false","NaN","Infinity","-Infinity"] { if self.literal(word) { return Ok(()); } }
                let start = self.pos;
                if self.peek() == Some('-') { self.pos += 1; }
                match self.peek() {
                    Some('0') => self.pos += 1,
                    Some('1'..='9') => { while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.pos += 1; } }
                    _ => { self.pos = start; return Err(("Expecting value", start)); }
                }
                if self.peek() == Some('.') && self.chars.get(self.pos+1).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.pos += 1; }
                }
                if matches!(self.peek(), Some('e' | 'E')) {
                    let exponent = self.pos;
                    self.pos += 1;
                    if matches!(self.peek(), Some('+' | '-')) { self.pos += 1; }
                    let digits = self.pos;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.pos += 1; }
                    if self.pos == digits { self.pos = exponent; }
                }
                Ok(())
            }
        }
    }
    fn container(&mut self, object: bool) -> Result<(), Error> {
        self.pos += 1;
        self.space();
        let end = if object { '}' } else { ']' };
        if self.peek() == Some(end) { self.pos += 1; return Ok(()); }
        loop {
            if object {
                if self.peek() != Some('"') { return Err(("Expecting property name enclosed in double quotes",self.pos)); }
                self.string()?;
                self.space();
                if self.peek() != Some(':') { return Err(("Expecting ':' delimiter",self.pos)); }
                self.pos += 1;
            }
            self.value()?;
            self.space();
            if self.peek() == Some(end) { self.pos += 1; return Ok(()); }
            if self.peek() != Some(',') { return Err(("Expecting ',' delimiter",self.pos)); }
            let comma = self.pos;
            self.pos += 1;
            self.space();
            if self.peek() == Some(end) { return Err((if object { "Illegal trailing comma before end of object" } else { "Illegal trailing comma before end of array" },comma)); }
        }
    }
}

pub(super) fn diagnostic(text: &str) -> String {
    let mut parser = Parser { chars: text.chars().collect(), pos: 0 };
    let result = if parser.peek() == Some('\u{feff}') { Err(("Unexpected UTF-8 BOM (decode using utf-8-sig)",0)) }
        else { parser.value().and_then(|()| { parser.space(); if parser.peek().is_some() { Err(("Extra data",parser.pos)) } else { Ok(()) } }) };
    match result {
        Ok(()) => String::new(),
        Err((detail,pos)) => {
            let prefix = &parser.chars[..pos];
            let line = 1 + prefix.iter().filter(|&&c| c == '\n').count();
            let column = pos + 1 - prefix.iter().rposition(|&c| c == '\n').map_or(0, |i| i+1);
            format!("{detail}: line {line} column {column} (char {pos})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_json_diagnostics_use_character_positions_and_python_313_syntax() {
        for (text, expected) in [
            ("{", "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"),
            ("[", "Expecting value: line 1 column 2 (char 1)"),
            ("[1,]", "Illegal trailing comma before end of array: line 1 column 3 (char 2)"),
            ("{\"a\":1,}", "Illegal trailing comma before end of object: line 1 column 7 (char 6)"),
            ("{\"a\" 1}", "Expecting ':' delimiter: line 1 column 6 (char 5)"),
            ("\"a\\q\"", "Invalid \\escape: line 1 column 3 (char 2)"),
            ("\"a\\uXX00\"", "Invalid \\uXXXX escape: line 1 column 4 (char 3)"),
            ("\"a\n\"", "Invalid control character at: line 1 column 3 (char 2)"),
            ("\"☃", "Unterminated string starting at: line 1 column 1 (char 0)"),
            ("[\"☃\",\n ?]", "Expecting value: line 2 column 2 (char 7)"),
            ("{} 1", "Extra data: line 1 column 4 (char 3)"),
            ("1e", "Extra data: line 1 column 2 (char 1)"),
            ("\u{feff}{}", "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"),
        ] { assert_eq!(diagnostic(text), expected, "{text:?}"); }
        for text in ["{}", " [true, false, null, -0.9e+20, NaN, Infinity, -Infinity] ", "{\"x\":\"\\uD800\"}"] { assert_eq!(diagnostic(text), ""); }
    }
}
