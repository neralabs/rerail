//! The regex helpers' calls reach their callees however far apart the code is mapped
//! (`rerail_compiler::native_cpu_codegen::relocations`).

use rerail_compiler::native_cpu_codegen::relocations::{Recording, assert_targets, separated_module};

#[test]
fn production_regex_helpers_reach_distant_callees() {
    let _recording = Recording::start();
    let mut module = separated_module();
    crate::code_rule_dsl::transducer::packed::native_cpu_regex::define_helpers(&mut module).unwrap();
    module.finalize_definitions().unwrap();
    assert!(assert_targets(&module) > 0);
}
