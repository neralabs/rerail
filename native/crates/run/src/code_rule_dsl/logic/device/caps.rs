//! The functions a device computes, by the native engine's names (decision 0287 §1): one table the planner's
//! placement (`plan::placement`) and a device's compile (Metal's `setfix::device::compile`) both read, so a
//! function the device gains is placed on it at once, and a function it lacks is named where the planner refuses it.
//!
//! The planner's terms name a function as `.rerail` does (`@substr`, `@add`); `native` gives the native engine's name
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
    /// A name's characters (`rr_strlen`, `sql_length`).
    Len,
    /// Where the second name first is in the first, in characters from 1; 0 where it is not (`sql_instr`).
    Instr,
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
    /// C's `%0Nd` (`@printf`'s lowering): a number's digits zero-padded to a width, the sign first.
    ZeroPad,
    Replace,
    /// A text less a prefix and a suffix (`str_between`), none where it has not both.
    StrBetween,
    /// A dotted name less its last parts (`pkg_up`).
    PkgUp,
    /// A node's text, read where its document's source lies (decision 0287 §2): at any length
    /// (`rr_node_text_any`), or past `text`'s inline bound alone, none shorter (`rr_node_text`, the long-text twin's).
    NodeTextAny,
    NodeText,
    /// A regex's capture group (`rx_group`): the text, then a constant regex and a constant group name.
    RxGroup,
    /// A regex's capture group by its number (`rx_extract`): the text and a constant regex, then the group's number.
    RxExtract,
    /// SQL's `trim(x)` and `ltrim(x)`: x less its spaces at both ends, or at its start.
    Trim,
    LTrim,
    /// SQL's `trim(x, y)`, `ltrim(x, y)`, `rtrim(x, y)`: x less the characters of y at both ends, its start, its end.
    TrimSet,
    LTrimSet,
    RTrimSet,
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
    /// A node's preorder key (`rr_node_order`, of its id: `node_order`'s `k`), read off its document's tape where it
    /// lies; a comparison of two nodes compares two of them (`lowering_tail::OrderNodes`).
    NodeOrder,
}

impl DeviceFn {
    /// The arguments the device's form takes.
    pub fn arity(self) -> usize {
        match self {
            DeviceFn::Arith(_) => 2,
            DeviceFn::Names { texts, .. } => texts,
            DeviceFn::Text { texts, nums, .. } => texts + nums,
            DeviceFn::RegexReplace => 3,
            DeviceFn::NodeOrder => 1,
        }
    }

    /// Whether the function's value is a text the device makes (into its arena).
    pub fn makes_text(self) -> bool {
        matches!(self, DeviceFn::Text { .. } | DeviceFn::RegexReplace)
    }
}

/// The native engine's name for `.rerail`'s function `f` (`strings::functions::native_call`), or `f` itself.
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
        "rr_strlen" | "sql_length" => Names { f: self::Names::Len, texts: 1 },
        "sql_instr" => Names { f: self::Names::Instr, texts: 2 },
        "lower" => text(TextFn::Lower, 1, 0),
        "sql_lower" | "ascii_lower" => text(TextFn::AsciiLower, 1, 0),
        "sql_upper" => text(TextFn::AsciiUpper, 1, 0),
        "head" => text(TextFn::Head, 1, 0),
        "dir" => text(TextFn::Dir, 1, 0),
        "rr_substr" => text(TextFn::Substr, 1, 2),
        "sql_substr" => text(TextFn::SqlSubstr, 1, 2),
        "slice" => text(TextFn::Slice, 1, 2),
        "rr_to_string" => text(TextFn::ToString, 0, 1),
        "zero_pad" => text(TextFn::ZeroPad, 0, 2),
        "sql_replace" => text(TextFn::Replace, 3, 0),
        "str_between" => text(TextFn::StrBetween, 3, 0),
        "pkg_up" => text(TextFn::PkgUp, 1, 1),
        "rr_node_text_any" => text(TextFn::NodeTextAny, 0, 1),
        "rr_node_text" => text(TextFn::NodeText, 0, 1),
        "rx_group" => text(TextFn::RxGroup, 3, 0),
        "rx_extract" => text(TextFn::RxExtract, 2, 1),
        "sql_trim" => text(TextFn::Trim, 1, 0),
        "sql_ltrim" => text(TextFn::LTrim, 1, 0),
        "sql_trim2" => text(TextFn::TrimSet, 2, 0),
        "sql_ltrim2" => text(TextFn::LTrimSet, 2, 0),
        "sql_rtrim2" => text(TextFn::RTrimSet, 2, 0),
        "rr_node_order" => NodeOrder,
        "replace_all" | "rx_replace" => RegexReplace,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `.rerail` name the planner's terms carry for a device function reaches its form by `native`, and each
    /// form takes as many arguments as the native engine's function (`strings::functions`), so the planner and the
    /// device read one table.
    #[test]
    fn the_planners_names_and_the_devices_are_one_table() {
        for (rr, form) in [("substr", "rr_substr"), ("to_string", "rr_to_string"), ("strlen", "rr_strlen"), ("add", "rr_add"), ("max", "rr_max")] {
            assert_eq!(native(rr), form);
            assert!(device_fn(native(rr)).is_some(), "{rr}");
        }
        for name in ["rr_add", "rr_sub", "rr_mul", "rr_div", "mod", "rr_max", "rr_min", "rr_strlen", "lower", "sql_lower", "sql_upper", "head", "dir",
                     "rr_substr", "sql_substr", "slice", "rr_to_string", "zero_pad", "sql_replace", "str_between", "pkg_up", "replace_all", "rx_replace", "str_cmp",
                     "rx_group", "rr_node_text_any", "rr_node_text", "rr_node_order", "sql_length", "sql_instr", "ascii_lower", "rx_extract", "sql_trim",
                     "sql_ltrim", "sql_trim2", "sql_ltrim2", "sql_rtrim2"] {
            let f = device_fn(name).expect(name);
            if let Some((_, kinds, _)) = crate::code_rule_dsl::logic::strings::functions::signature(name) {
                assert_eq!(kinds.len(), f.arity(), "{name}");
            }
        }
    }
}
