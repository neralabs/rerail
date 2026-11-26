fn total(items: &[Item]) -> u64 {
    let mut sum = 0;
    for v in items {
        if v.active {
            sum += v.price * v.count;
        }
    }
    sum
}
