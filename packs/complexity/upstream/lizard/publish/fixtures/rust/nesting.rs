// Functions in impl blocks and traits, a nested fn scored on its own, and closures, whose decisions count toward the
// fn around them as lizard counts them (it does not read a closure as a function).

pub struct Grader {
    limit: u32,
    strict: bool,
}

impl Grader {
    pub fn new(limit: u32, strict: bool) -> Self {
        let limit = if limit > 100 { 100 } else { limit };
        let strict = strict && limit > 0;
        Grader { limit, strict }
    }

    pub fn grade(&self, score: u32, late: bool) -> &'static str {
        if score > 90 && !late {
            "A"
        } else if score > 80 || self.strict {
            "B"
        } else if score > 70 {
            "C"
        } else if score > self.limit {
            "D"
        } else {
            "F"
        }
    }

    pub async fn scale(&self, scores: Vec<u32>) -> Vec<u32> {
        scores
            .into_iter()
            .map(|score| if score > self.limit { self.limit } else { score })
            .filter(|score| *score > 0 && (*score % 2 == 0 || self.strict))
            .collect()
    }
}

pub trait Clamp {
    // A required method has no body, so it is no function here; lizard runs it on into the next one and scores that
    // under its name and line.
    fn low(&self) -> i32;

    fn clamp(&self, value: i32, high: i32) -> i32 {
        if value < self.low() {
            self.low()
        } else if value > high {
            high
        } else {
            value
        }
    }
}

pub fn outer_with_complex_inner(codes: &[u32]) -> Vec<&'static str> {
    fn label(code: u32) -> &'static str {
        if code == 1 {
            "one"
        } else if code == 2 {
            "two"
        } else if code == 3 {
            "three"
        } else if code == 4 {
            "four"
        } else if code == 5 {
            "five"
        } else if code == 6 {
            "six"
        } else {
            "many"
        }
    }
    codes.iter().map(|code| label(*code)).collect()
}

pub const fn konst(flag: bool) -> u32 {
    if flag { 1 } else { 0 }
}

pub unsafe extern "C" fn exported(value: i32) -> i32 {
    if value > 0 { value } else { 0 }
}
