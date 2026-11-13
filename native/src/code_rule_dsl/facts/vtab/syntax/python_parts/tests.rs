    use super::with_offsets;

    /// Decoded parts and their source byte offsets, relative to the part's literal token.
    fn str_parts_with_offsets<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8, usize)> {
        with_offsets(parts)
    }

    /// `str_part(lit, i, v, at, spec)`: the str constants CPython builds from Python literal `lit` (a `string`, or a
    /// `concatenated_string`'s parts `parts`, each with its node): plain runs join across adjacent literals; an
    /// f-string's fields split them; a format spec's literal runs are pieces of their own (`spec` 1). Bytes and
    /// t-strings give none. Returns (v, at, spec) per piece, in order.
    fn str_parts<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8)> {
        with_offsets(parts).into_iter().map(|(v, n, spec, _)| (v, n, spec)).collect()
    }

    #[test]
    fn decoded_string_constants_keep_precise_byte_starts() {
        let parts = str_parts_with_offsets(&[(0, "f\"\"\"json\n{a}\njson\"\"\"")]);
        assert_eq!(parts.iter().map(|p| p.3).collect::<Vec<_>>(), [4, 12]);
        let parts = str_parts_with_offsets(&[(0, "f'λ{x}μ'")]);
        assert_eq!(parts.iter().map(|p| p.3).collect::<Vec<_>>(), [2, 7]);
        assert_eq!(str_parts_with_offsets(&[(0, "'plain'")])[0].3, 0);
    }

    #[test]
    fn str_parts_follow_cpython() {
        let parts = |lits: &[&str]| str_parts(&lits.iter().enumerate().map(|(i, t)| (i, *t)).collect::<Vec<_>>());
        assert_eq!(parts(&["'a'", "\"b\\n\""]), [("ab\n".to_owned(), 0, 0)]);
        assert_eq!(parts(&["f'x{y}z{{'"]), [("x".to_owned(), 0, 0), ("z{".to_owned(), 0, 0)]);
        assert_eq!(parts(&["f'{x:>{w}d}'"]), [(">".to_owned(), 0, 1), ("d".to_owned(), 0, 1)]);
        assert_eq!(parts(&["f'{x=}'"]), [("x=".to_owned(), 0, 0)]);
        assert!(parts(&["b'x'"]).is_empty());
    }
