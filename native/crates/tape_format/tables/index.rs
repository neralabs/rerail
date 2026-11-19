// The tables the lab writes (`tape16 table`), by tape tag. Regenerated with them.
pub fn text(tag: u8) -> Option<&'static str> {
    match tag {
        1 => Some(include_str!("1.tsv")),
        2 => Some(include_str!("2.tsv")),
        3 => Some(include_str!("3.tsv")),
        6 => Some(include_str!("6.tsv")),
        7 => Some(include_str!("7.tsv")),
        8 => Some(include_str!("8.tsv")),
        9 => Some(include_str!("9.tsv")),
        10 => Some(include_str!("10.tsv")),
        12 => Some(include_str!("12.tsv")),
        13 => Some(include_str!("13.tsv")),
        14 => Some(include_str!("14.tsv")),
        _ => None,
    }
}
