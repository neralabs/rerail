"""Port boundary collectors, raw scan counts, category projections, and reporting filters."""


def async_sync_boundary(source, target, header):
    return header('async_sync_boundary', target) + '''
.violation Count(at: span, count: number) code "BS_COUNT" "{count}"
Count(@place("<inventory>", 1), n) :- option("entryPoint", "scan_tree"), n = count : { option_item("checkedPaths", _, _) }.
.violation Category(at: span, index: number) code "BS_CATEGORY" "category"
Category(@place("<category>", 1), i) :- option("entryPoint", "category"), option("category", c), option_item("categories", i, c).
'''
