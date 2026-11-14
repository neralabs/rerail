//! One key to many values (decision 0173 §4): a CSR base, as built, and a tail of 16-value blocks for what is
//! appended since, chained from each key's last block. An append fills its key's tail block or starts one; it never
//! moves the base. `freeze` lays both out as one CSR, as a built image stores it.

const BLOCK: usize = 16;

#[derive(Clone, Copy)]
struct Block {
    len: u32,
    /// The next block + 1 (0: none).
    next: u32,
    values: [u32; BLOCK],
}

#[derive(Default)]
pub struct Grouped {
    /// The base: key k's values are `values[starts[k]..starts[k + 1]]`.
    starts: Vec<u32>,
    values: Vec<u32>,
    /// Per key, its first and last tail block + 1 (0: none).
    first: Vec<u32>,
    last: Vec<u32>,
    blocks: Vec<Block>,
}

impl Grouped {
    /// A base of `starts` and `values` (CSR), with no tails.
    pub fn of(starts: Vec<u32>, values: Vec<u32>) -> Grouped {
        let keys = starts.len().saturating_sub(1);
        Grouped { starts, values, first: vec![0; keys], last: vec![0; keys], blocks: Vec::new() }
    }
    pub fn keys(&self) -> usize {
        self.first.len()
    }
    /// Appends `value` to key `key`'s values: its tail block, or a new one.
    pub fn push(&mut self, key: u32, value: u32) {
        let k = key as usize;
        if k >= self.first.len() {
            self.first.resize(k + 1, 0);
            self.last.resize(k + 1, 0);
        }
        let last = self.last[k];
        if last != 0 && (self.blocks[last as usize - 1].len as usize) < BLOCK {
            let b = &mut self.blocks[last as usize - 1];
            b.values[b.len as usize] = value;
            b.len += 1;
            return;
        }
        let mut b = Block { len: 1, next: 0, values: [0; BLOCK] };
        b.values[0] = value;
        self.blocks.push(b);
        let id = self.blocks.len() as u32;
        if last == 0 {
            self.first[k] = id;
        } else {
            self.blocks[last as usize - 1].next = id;
        }
        self.last[k] = id;
    }
    fn base(&self, k: usize) -> &[u32] {
        if k + 1 < self.starts.len() { &self.values[self.starts[k] as usize..self.starts[k + 1] as usize] } else { &[] }
    }
    /// Key `key`'s values: its base, then its tail blocks, in order.
    pub fn get(&self, key: u32) -> impl Iterator<Item = u32> + '_ {
        let k = key as usize;
        let base = self.base(k).iter().copied();
        let mut block = self.first.get(k).copied().unwrap_or(0);
        let tail = std::iter::from_fn(move || {
            if block == 0 {
                return None;
            }
            let b = &self.blocks[block as usize - 1];
            block = b.next;
            Some(b.values[..b.len as usize].iter().copied())
        })
        .flatten();
        base.chain(tail)
    }
    pub fn tail_blocks(&self) -> usize {
        self.blocks.len()
    }
    /// The whole index as one CSR over `keys` keys (at least as many as were pushed): no tails.
    pub fn freeze(&self, keys: usize) -> (Vec<u32>, Vec<u32>) {
        let keys = keys.max(self.keys());
        let mut starts = Vec::with_capacity(keys + 1);
        let mut values = Vec::with_capacity(self.values.len() + self.blocks.len() * BLOCK);
        for k in 0..keys {
            starts.push(values.len() as u32);
            values.extend(self.get(k as u32));
        }
        starts.push(values.len() as u32);
        (starts, values)
    }
}
