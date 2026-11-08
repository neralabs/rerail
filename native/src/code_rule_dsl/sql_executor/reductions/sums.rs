//! Keep unused sums exact; enforce the public integer range when a context reads one.
use super::*;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};

fn wide(high: i64, low: i64) -> i128 {
    (i128::from(high) << 64) | i128::from(low as u64)
}
struct Part(bool);
impl Aggregate<i128, i64> for Part {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<i128> {
        Ok(0)
    }
    fn step(&self, ctx: &mut Context<'_>, sum: &mut i128) -> rusqlite::Result<()> {
        let low: i64 = ctx.get(0)?;
        let high: Option<i64> = ctx.get(2)?;
        let value = high.map_or(i128::from(low), |high| wide(high, low));
        *sum = value
            .checked_mul(i128::from(ctx.get::<i64>(1)?))
            .and_then(|value| sum.checked_add(value))
            .ok_or_else(|| {
                rusqlite::Error::UserFunctionError("retained sum exceeds 128 bits".into())
            })?;
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, sum: Option<i128>) -> rusqlite::Result<i64> {
        let sum = sum.unwrap_or(0);
        Ok(if self.0 {
            (sum >> 64) as i64
        } else {
            sum as i64
        })
    }
}
pub(super) fn register(db: &rusqlite::Connection) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_aggregate_function(c"rr_sum_low", 3, flags, Part(false))
        .map_err(error)?;
    db.create_aggregate_function(c"rr_sum_high", 3, flags, Part(true))
        .map_err(error)?;
    db.create_scalar_function(c"rr_sum_value", 2, flags, |ctx| {
        i64::try_from(wide(ctx.get(0)?, ctx.get(1)?))
            .map_err(|_| rusqlite::Error::UserFunctionError("integer overflow".into()))
    })
    .map_err(error)
}
