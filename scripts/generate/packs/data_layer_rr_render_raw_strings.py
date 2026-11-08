"""Translate raw render-string classification and reporting."""
def render_raw_strings(source,target,header):
    return header('render_raw_strings',target)+r'''
.violation RSFact(at: span, method_name: symbol, class_name: symbol, has_class: number, ordinal: number, violation_type: symbol, preview: symbol) code "RS_FACT" "{preview}"
RSFact(@place(path, 1), "render", "", 0, 0, "string_literal", "disabled") :- doc(_, path, _), 1 = 0.
.violation RSReport(at: span, render_methods_scanned: number, violation_count: number, include_report: number) code "RS_REPORT" "report"
RSReport(@place(path, 0), 0, 0, 0) :- doc(d, path, _), doc_role(d, "primary"), option("entryPoint", _).
.violation RSRead(at: span) code "RS_READ" "read"
RSRead(@place("", 0)) :- option("entryPoint", "source_gate"), option("providedSource", source), contains("render", source).
'''
