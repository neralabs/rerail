//! Conservative SQL relation-use counts after the authorizer establishes dependencies.
pub(super) fn occurrences(query: &str, name: &str) -> usize {
    static IDENTIFIER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("SQL identifiers")
    });
    // The authorizer supplies the actual read set. Textual counts can overestimate
    // references in literals or local aliases, which only adds a conservative cut.
    IDENTIFIER
        .find_iter(query)
        .filter(|token| token.as_str().eq_ignore_ascii_case(name))
        .count()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_qualifiers_and_literal_mentions_do_not_invent_second_readers() {
        assert_eq!(occurrences("SELECT facts.a,facts.b FROM facts WHERE facts.c=1","facts"),1);
        assert_eq!(occurrences("SELECT 'facts' label,f.a FROM facts f /* facts */ -- facts\nWHERE f.a='it''s facts'","facts"),1);
        assert_eq!(occurrences("SELECT \"facts\".a,[facts].b FROM \"facts\"","facts"),1);
    }
    #[test]
    fn real_repeated_relations_keep_their_index_boundary() {
        assert_eq!(occurrences("SELECT a.x FROM facts a JOIN facts b ON a.x=b.x","facts"),2);
        assert_eq!(occurrences("SELECT * FROM facts f WHERE EXISTS(SELECT 1 FROM facts g WHERE g.x=f.x)","facts"),2);
    }
}
