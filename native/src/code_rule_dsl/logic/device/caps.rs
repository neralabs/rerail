//! The functions a device computes, by the native engine's names (decision 0287 §1): one table the planner's
//! placement (`plan::placement`) and a device's compile (Metal's `setfix::device::compile`) both read, so a
//! function the device gains is placed on it at once, and a function it lacks is named where the planner refuses it.
//!
//! The planner's terms name a function as `.rr` does (`@substr`, `@add`); `native` gives the native engine's name
//! for it (`rr_substr`, `rr_add`), the name the compiled steps carry. The table names no device's operation codes: a
//! device maps each entry to its own (Metal: an exhaustive match in `setfix::device`, so an entry without an
//! operation does not compile).

/// An integer operation of two numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Max,
    Min,
}

/// A function of names, compared or counted where the names are (never made).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Names {
    /// The order of two names (`str_cmp`).
    Cmp,
    /// A name's characters (`rr_strlen`).
    Len,
}

/// A text the device makes into its text arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFn {
    Lower,
    AsciiLower,
    AsciiUpper,
    Head,
    Dir,
    Substr,
    SqlSubstr,
    Slice,
    ToString,
    Replace,
}

/// What a device computes a function with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFn {
    Arith(Arith),
    /// Of `texts` texts.
    Names { f: Names, texts: usize },
    /// Of `texts` texts, then `nums` numbers.
    Text { f: TextFn, texts: usize, nums: usize },
    /// A regex's replacement in a text: the text, then a constant regex and a constant replacement.
    RegexReplace,
}

impl DeviceFn {
    /// The arguments the device's form takes.
    pub fn arity(self) -> usize {
        match self {
            DeviceFn::Arith(_) => 2,
            DeviceFn::Names { texts, .. } => texts,
            DeviceFn::Text { texts, nums, .. } => texts + nums,
            DeviceFn::RegexReplace => 3,
        }
    }

    /// Whether the function's value is a text the device makes (into its arena).
    pub fn makes_text(self) -> bool {
        matches!(self, DeviceFn::Text { .. } | DeviceFn::RegexReplace)
    }
}

/// The native engine's name for `.rr`'s function `f` (`strings::functions::native_call`), or `f` itself.
pub fn native(f: &str) -> &str {
    crate::code_rule_dsl::logic::strings::functions::native_call(f).map_or(f, |(n, _, _)| n)
}

/// The device's form of the native function `name`, none for a function it does not compute.
pub fn device_fn(name: &str) -> Option<DeviceFn> {
    use DeviceFn::*;
    let text = |f, texts, nums| Text { f, texts, nums };
    Some(match name {
        "rr_add" => Arith(self::Arith::Add),
        "rr_sub" => Arith(self::Arith::Sub),
        "rr_mul" => Arith(self::Arith::Mul),
        "rr_div" => Arith(self::Arith::Div),
        "mod" => Arith(self::Arith::Mod),
        "rr_max" | "max" => Arith(self::Arith::Max),
        "rr_min" | "min" => Arith(self::Arith::Min),
        "str_cmp" => Names { f: self::Names::Cmp, texts: 2 },
        "rr_strlen" => Names { f: self::Names::Len, texts: 1 },
        "lower" => text(TextFn::Lower, 1, 0),
        "sql_lower" => text(TextFn::AsciiLower, 1, 0),
        "sql_upper" => text(TextFn::AsciiUpper, 1, 0),
        "head" => text(TextFn::Head, 1, 0),
        "dir" => text(TextFn::Dir, 1, 0),
        "rr_substr" => text(TextFn::Substr, 1, 2),
        "sql_substr" => text(TextFn::SqlSubstr, 1, 2),
        "slice" => text(TextFn::Slice, 1, 2),
        "rr_to_string" => text(TextFn::ToString, 0, 1),
        "sql_replace" => text(TextFn::Replace, 3, 0),
        "replace_all" | "rx_replace" => RegexReplace,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `.rr` name the planner's terms carry for a device function reaches its form by `native`, and each
    /// form takes as many arguments as the native engine's function (`strings::functions`), so the planner and the
    /// device read one table.
    #[test]
    fn the_planners_names_and_the_devices_are_one_table() {
        for (rr, form) in [("substr", "rr_substr"), ("to_string", "rr_to_string"), ("strlen", "rr_strlen"), ("add", "rr_add"), ("max", "rr_max")] {
            assert_eq!(native(rr), form);
            assert!(device_fn(native(rr)).is_some(), "{rr}");
        }
        for name in ["rr_add", "rr_sub", "rr_mul", "rr_div", "mod", "rr_max", "rr_min", "rr_strlen", "lower", "sql_lower", "sql_upper", "head", "dir",
                     "rr_substr", "sql_substr", "slice", "rr_to_string", "sql_replace", "replace_all", "rx_replace", "str_cmp"] {
            let f = device_fn(name).expect(name);
            if let Some((_, kinds, _)) = crate::code_rule_dsl::logic::strings::functions::signature(name) {
                assert_eq!(kinds.len(), f.arity(), "{name}");
            }
        }
        assert_eq!(device_fn(native("rx_group")), None);
    }
}
