"""Translate render-function contexts, decoded f-string previews, and reports."""
def render_fstrings(source, target, header):
    return header('render_fstrings', target) + r'''
.violation RFFact(at: span, method_name: symbol, class_name: symbol, has_class: number, ordinal: number) code "RF_FACT" "disabled"
RFFact(@place(path, 1), "render", "", 0, 0) :- doc(_, path, _), 1 = 0.
.violation RFReport(at: span, render_methods_scanned: number, violation_count: number) code "RF_REPORT" "report"
RFReport(@place(path, 0), 0, 0) :- doc(d, path, _), doc_role(d, "primary"), option("entryPoint", _).
'''
