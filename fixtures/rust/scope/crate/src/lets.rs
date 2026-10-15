// Rerail-authored fixture (Apache-2.0).
pub fn shadowing(mut input: i32) -> i32 {
    let x = input;
    let x = x + 1;
    {
        let x = x + 10;
        input += x;
    }
    let y;
    y = x;
    input + y
}

pub fn nested() -> i32 {
    let value = 3;
    let answer = {
        let value = value + 2;
        value
    };
    answer
}

pub fn patterns((mut a, ref b): (i32, i32), c: Option<i32>) -> i32 {
    let Some(c) = c else { return a };
    a += *b;
    a + c
}

pub fn loops(values: &[i32]) -> i32 {
    let mut total = 0;
    for &item in values {
        total += item;
    }
    let mut i = 0;
    while let Some(v) = values.get(i) {
        total += *v;
        i += 1;
        if i > 2 { break; }
    }
    total
}

pub fn let_else(value: Option<i32>) -> i32 {
    let Some(number) = value else { return 0 };
    number
}
