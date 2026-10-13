// Rerail-authored fixture (Apache-2.0).
#[derive(Clone, Copy)]
pub enum Signal { Stop, Move(i32), Pair { left: i32, right: i32 } }

pub fn match_bind(signal: Signal) -> i32 {
    match signal {
        Signal::Stop => 0,
        Signal::Move(n @ 1..=10) => n,
        Signal::Move(n) if n > 10 => n,
        Signal::Move(_) => 0,
        Signal::Pair { left: a, right: b } => a + b,
    }
}

pub fn or_patterns(value: Result<i32, i32>) -> i32 {
    match value {
        Ok(x) | Err(x) => x,
    }
}

pub fn conditional(value: Option<(i32, i32)>) -> i32 {
    if let Some((first, second)) = value {
        first + second
    } else { 0 }
}

pub fn labelled(values: &[i32]) -> i32 {
    let mut total = 0;
    'outer: for value in values {
        for other in values {
            if value == other { continue 'outer; }
            total += value + other;
        }
    }
    total
}

pub fn destructure(slice: &[i32]) -> i32 {
    match slice {
        [head, tail @ ..] => *head + tail.len() as i32,
        [] => 0,
    }
}
