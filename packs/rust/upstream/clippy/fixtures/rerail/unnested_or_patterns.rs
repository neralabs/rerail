#![warn(clippy::unnested_or_patterns)]

struct T5(u8, u8, u8, u8, u8);
struct T6(u8, u8, u8, u8, u8, u8);

fn tuples(a: (u8, u8, u8, u8, u8), b: (u8, u8, u8, u8, u8, u8)) {
    if let (1, 2, 3, 4, 5) | (1, 2, 3, 4, 6) = a {}
    //~^ unnested_or_patterns
    if let (1, 2, 3, 4, 5, 6) | (1, 2, 3, 4, 5, 7) = b {}
    //~^ unnested_or_patterns
    // two elements differ: not nestable
    if let (1, 2, 3, 4, 5) | (1, 2, 3, 6, 6) = a {}
}

fn slices(a: &[u8], b: &[u8]) {
    if let [1, 2, 3, 4, 5] | [1, 2, 3, 4, 6] = a {}
    //~^ unnested_or_patterns
    if let [1, 2, 3, 4, 5, 6] | [1, 2, 3, 4, 5, 7] = b {}
    //~^ unnested_or_patterns
    if let [1, 2, 3, 4, 5] | [1, 2, 3, 6, 6] = a {}
}

fn tuple_structs(a: T5, b: T6) {
    if let T5(1, 2, 3, 4, 5) | T5(1, 2, 3, 4, 6) = a {}
    //~^ unnested_or_patterns
    if let T6(1, 2, 3, 4, 5, 6) | T6(1, 2, 3, 4, 5, 7) = b {}
    //~^ unnested_or_patterns
    if let T5(1, 2, 3, 4, 5) | T5(1, 2, 3, 6, 6) = a {}
}
