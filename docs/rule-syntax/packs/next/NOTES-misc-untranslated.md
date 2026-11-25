# Batch misc-untranslated: next

## next-no-page-custom-font: same

check-rr refused revision 2: `default_component: match_all over $v, a node or span not bound by source, src_line, kind
or a capture`. The third `default_component` clause ran `match` over the export's node. It now reads the node's
`text` and matches that: the same test (the first identifier token after `export default`, the plugin's token scan).

Final ptcheck: same (fixtures 11 both; examples and samples-next have no findings on either side).

Library changes: none. Engine gaps: none found.
