// Rerail-authored fixture (Apache-2.0).
pub struct Widget { pub value: i32 }
pub struct Pair(pub i32, pub i32);
pub union Bits { pub integer: u32, pub bytes: [u8; 4] }
pub enum Choice { A, B(i32) }
pub trait Measure { fn measure(&self) -> i32; }
pub type Alias = Widget;
pub const LIMIT: i32 = 8;
pub static GLOBAL: i32 = 9;

pub fn Widget() -> i32 { LIMIT }
pub fn helper(value: i32) -> i32 { value + GLOBAL }

impl Measure for Widget {
    fn measure(&self) -> i32 { self.value * 2 }
}

impl Widget {
    pub fn new(value: i32) -> Self { Self { value } }
    pub fn value(&self) -> i32 { self.value }
}

pub fn use_items() -> i32 {
    let widget = Widget::new(3);
    let pair = Pair(1, 2);
    let choice = Choice::B(pair.0);
    match choice { Choice::B(v) => widget.measure() + v, Choice::A => 0 }
}

pub fn later() -> i32 {
    call_later();
    fn call_later() {}
    1
}
