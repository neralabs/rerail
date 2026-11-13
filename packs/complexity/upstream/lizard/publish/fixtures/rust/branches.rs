// Expression decisions: if/else if (and if let), for, while (and while let), match once however many arms, a match
// arm's guard, the ? operator, && and ||, and the && of a let chain. loop is no decision.

pub fn if_else_at_limit(x: u32) -> &'static str {
    if x == 1 {
        "one"
    } else if x == 2 {
        "two"
    } else if x == 3 {
        "three"
    } else if x == 4 {
        "four"
    } else if x == 5 {
        "five"
    } else {
        "many"
    }
}

pub fn if_else_over_limit(x: u32, label: Option<&str>) -> &str {
    if x == 1 {
        "one"
    } else if x == 2 {
        "two"
    } else if x == 3 {
        "three"
    } else if x == 4 {
        "four"
    } else if x == 5 {
        "five"
    } else if let Some(text) = label {
        text
    } else {
        "many"
    }
}

pub fn loops_at_limit(rows: &[Vec<u32>], mut queue: Vec<u32>) -> u32 {
    let mut total = 0;
    for row in rows {
        total += row.len() as u32;
    }
    while total > 100 {
        total -= 1;
    }
    while let Some(next) = queue.pop() {
        total += next;
    }
    loop {
        total += 1;
        if total > 10 {
            break;
        }
    }
    for index in 0..3 {
        total += index;
    }
    total
}

pub fn loops_over_limit(rows: &[Vec<u32>], mut queue: Vec<u32>) -> u32 {
    let mut total = 0;
    for row in rows {
        total += row.len() as u32;
    }
    while total > 100 {
        total -= 1;
    }
    while let Some(next) = queue.pop() {
        total += next;
    }
    'outer: loop {
        total += 1;
        if total > 10 {
            break 'outer;
        }
    }
    for index in 0..3 {
        total += index;
    }
    for index in 0..2 {
        total += index;
    }
    total
}

pub fn match_counts_once(code: u8) -> &'static str {
    match code {
        1 => "alpha",
        2 => "bravo",
        3 => "charlie",
        4 => "delta",
        5 => "echo",
        6 => "foxtrot",
        _ => "unknown",
    }
}

pub fn nested_matches(outer: u8, inner: u8) -> &'static str {
    match outer {
        1 => match inner {
            2 => "xp",
            _ => "x",
        },
        _ => "none",
    }
}

pub fn match_over_limit(code: i32, loud: bool) -> &'static str {
    match code {
        n if n > 100 => "huge",
        n if n > 10 && loud => "BIG",
        n if n > 10 => "big",
        n if n < 0 || n == -1 => "negative",
        0 => "zero",
        _ => "small",
    }
}

pub fn try_over_limit(raw: &[&str]) -> Result<i64, std::num::ParseIntError> {
    let a: i64 = raw[0].parse()?;
    let b: i64 = raw[1].parse()?;
    let c: i64 = raw[2].parse()?;
    let d: i64 = raw[3].parse()?;
    let e: i64 = raw[4].parse()?;
    let f: i64 = raw[5].parse()?;
    Ok(a + b + c + d + e + f)
}

pub fn logical_at_limit(a: bool, b: bool, c: bool, d: bool, e: bool, f: bool) -> bool {
    a && b || c && d || e && f
}

pub fn logical_over_limit(a: bool, b: bool, c: bool, d: bool, e: bool, f: bool, g: bool) -> bool {
    a && b || c && d || e && f || g
}

pub fn let_chain_over_limit(first: Option<u32>, second: Option<u32>, ready: bool) -> u32 {
    if let Some(a) = first && let Some(b) = second && ready && a > b {
        return a - b;
    }
    if let Some(a) = first && a > 10 {
        return a;
    }
    0
}
