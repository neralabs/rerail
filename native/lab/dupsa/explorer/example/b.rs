impl Cart {
    fn total(&self) -> u64 {
        let mut sum = 0;
        // only the active ones
        for x in &self.items {
            if x.active {
                sum += x.price * x.count;
            }
        }
        sum
    }
}
