//! Random reads of one column per frozen encoding (decision 0622), against the raw column: nanoseconds a read.
use rerail_blocks::block::{Block, kind};
use rerail_blocks::rows::Rows;
use std::sync::Mutex;

fn main() {
    let n = 1u32 << 20;
    let cols: [(&str, fn(i64) -> i64); 6] = [
        ("const", |_| 7),
        ("pack", |i| (i * 2_654_435_761) % 1000),
        ("runs", |i| i / 300 * 11),
        ("line", |i| i * 3 + (i * 7) % 5),
        ("groups", |i| (i >> 7) * 100_000 + (i * 2_654_435_761) % 64),
        ("raw", |i| (i * 2_654_435_761) ^ (i << 45)),
    ];
    let mut order: Vec<u32> = (0..n).map(|i| (u64::from(i) * 2_654_435_761 % u64::from(n)) as u32).collect();
    order.truncate(1 << 20);
    for (sealed, label) in [(false, "raw chunks"), (true, "sealed")] {
        let block = Block::anon(kind::FACTS, 256 << 20).with_free_list();
        let lock = Mutex::new(());
        let rows: &Rows = unsafe { &*block.alloc_t::<Rows>(1) };
        unsafe { Rows::init(std::ptr::from_ref(rows).cast_mut(), &block, &[8; 6], 4) };
        let s = rows.claim(&block, &lock, n);
        for i in 0..n {
            for (c, (_, f)) in cols.iter().enumerate() {
                unsafe { rows.put(s + i, c, f(i64::from(i))) };
            }
        }
        if sealed {
            unsafe { rows.seal(&block) };
        }
        for (c, (name, _)) in cols.iter().enumerate() {
            let r = rows.reader(c);
            let (mut best, mut acc) = (f64::MAX, 0i64);
            for _ in 0..5 {
                let t = std::time::Instant::now();
                for &i in &order {
                    acc = acc.wrapping_add(r.get(i));
                }
                best = best.min(t.elapsed().as_nanos() as f64 / order.len() as f64);
            }
            let d = rows.col(c);
            let tags: std::collections::BTreeSet<&str> = (0..rerail_blocks::rows::DIRS).filter_map(|k| d.sealed(k)).map(|ic| ic.tag_name()).collect();
            println!("{label:10} {name:7} {best:6.2} ns a read  {tags:?} ({})", acc & 1);
        }
    }
}
