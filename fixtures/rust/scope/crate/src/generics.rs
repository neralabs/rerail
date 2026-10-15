// Rerail-authored fixture (Apache-2.0).
struct Holder<'a, T, const N: usize> { value: &'a T, data: [u8; N] }

pub fn generic<'outer, T: Copy, const SIZE: usize>(value: T, input: &'outer [u8; SIZE]) -> T {
    let _holder = Holder::<T, SIZE> { value: &value, data: *input };
    value
}

pub fn shadow<'a, T>(outer: &'a T) -> &'a T {
    fn inner<'a, T>(value: &'a T) -> &'a T { value }
    inner(outer)
}

pub trait Factory<T> { type Output; const COUNT: usize; fn make(&self, value: T) -> Self::Output; }

impl<T: Clone> Factory<T> for Holder<'_, T, 1> {
    type Output = T;
    const COUNT: usize = 1;
    fn make(&self, value: T) -> Self::Output { value.clone() }
}

pub fn associated<T: Clone>(value: T) -> T {
    let holder = Holder { value: &value, data: [0] };
    <Holder<'_, T, 1> as Factory<T>>::make(&holder, value.clone())
}
