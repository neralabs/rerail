    use super::*;

    /// Node `i` read from the image's bytes in place, as the device reads it.
    fn read(bytes: &[u8], layout: &Layout, codes: &[DeviceCode], i: usize) -> (DeviceCode, u32, u32, u32) {
        let code = codes[cell_at(bytes, layout.code, 2, i) as usize];
        let start = |i: usize| cell_at(bytes, layout.base, 4, i / CHECKPOINT_STRIDE) + cell_at(bytes, layout.off, layout.widths[1], i);
        let size = size_at(bytes, layout, i);
        let tail = cell_at(bytes, layout.tail, layout.widths[2], i);
        let end = if layout.from_start { start(i) } else { start(i + size as usize - 1) } + tail;
        (code, start(i), end, size)
    }

    #[test]
    fn an_image_read_in_place_is_the_restored_tape() {
        for (source, lang) in [
            ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
            ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
            ("fn main() { let x: Vec<u8> = vec![1, 2]; if let Some(y) = x.first() { println!(\"{y}\"); } }\n", GrammarDialect::Rust),
            ("def broken(:\n    pass\n", GrammarDialect::Python),
        ] {
            let grammar = Grammar::new(lang).unwrap();
            let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
            let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
            let tape = TapeDoc::restore(Arc::new(source.to_owned()), Arc::clone(&grammar), &image, &|| false).unwrap();
            let bytes = tape.resident_parts().unwrap().0;
            let layout = layout(&bytes).unwrap();
            for column in [layout.code, layout.size, layout.base, layout.off, layout.tail] {
                assert_eq!(column % COLUMN_ALIGN, 0, "{lang:?}: columns are aligned");
            }
            assert_eq!(layout.nodes as usize, tape.transducer_len());
            assert_eq!(layout.source_len, source.len());
            let codes = device_codes(&bytes, &layout, &grammar).unwrap();
            let mut parents_column = vec![0; layout.nodes as usize];
            parents(&bytes, &layout, &mut parents_column).unwrap();
            for i in 0..layout.nodes {
                let (code, start, end, size) = read(&bytes, &layout, &codes, i as usize);
                assert_eq!(code.kind, tape.transducer_kind(i), "{lang:?} node {i} kind");
                assert_eq!(code.field, tape.transducer_field(i), "{lang:?} node {i} field");
                assert_eq!(code.flags, tape.transducer_flags(i), "{lang:?} node {i} flags");
                assert_eq!(start as usize..end as usize, tape.transducer_span(i), "{lang:?} node {i} span");
                assert_eq!(i + 1..i + size, tape.transducer_descendants(i), "{lang:?} node {i} size");
                assert_eq!((parents_column[i as usize] != NONE).then_some(parents_column[i as usize]), tape.transducer_parent(i), "{lang:?} node {i} parent");
                let classes = tape.transducer_supertypes().filter(|&(node, _)| node == i).map(|(_, class)| class).collect::<Vec<_>>();
                assert_eq!(&code.classes[..usize::from(code.class_count)], classes.as_slice(), "{lang:?} node {i} classes");
            }
        }
    }
