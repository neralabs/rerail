"""Static API-to-policy routing; no expected result influences the selected program."""


def policy_filename(check, entry):
    if check=='unresolved_forward_refs' and (entry=='cli_selection' or (entry or '').startswith('_uf_cli_')):
        return 'unresolved-forward-refs-cli.rr'
    if check == 'serverplan_state_tripwires' and entry is not None:
        return 'serverplan-state-tripwires-source.rr'
    if check == 'legacy_execution_surfaces' and entry in {'scan_repo', '_iter_python_files'}:
        return 'legacy-execution-surfaces-repository.rr'
    if check=='dynamic_module_loading' and entry=='_report_facts':
        return 'dynamic-module-loading-facts.rr'
    if check=='dynamic_module_loading' and entry in {'_dynamic_cli_paths','_dynamic_plugin_metadata'}:
        return 'dynamic-module-loading-paths.rr'
    if check=='dynamic_module_loading' and entry in {'_dynamic_repository','_dynamic_disk_file'}:
        return 'dynamic-module-loading-repository.rr'
    if check=='plan_composition_architecture' and entry=='scan_semantic_source':
        return 'plan-composition-architecture-source.rr'
    if check=='serverplan_grammar_surface' and entry in {'_project_facts','_project_messages'}:
        return 'serverplan-grammar-surface-project.rr'
    if check == 'impl_direct_execution' and entry.startswith('_ratchet_'):
        return 'impl-direct-execution-ratchet.rr'
    if check == 'serverplan_steps' and entry in {'_sign_request','_sign_error','_sign_update'}:
        return 'serverplan-steps-signing.rr'
    if check == 'serverplan_steps' and entry == '_module_settings':
        return 'serverplan-steps-settings.rr'
    if check == 'serverplan_steps' and entry == '_cli_paths':
        return 'serverplan-steps-paths.rr'
    if check == 'serverplan_steps' and entry in {
            '_cli_report', '_cli_legacy_report', '_cli_signature_failure'}:
        return 'serverplan-steps-report.rr'
    if check == 'serverplan_steps' and entry == 'scan_legacy_postop_violations':
        return 'serverplan-steps-legacy.rr'
    if check == 'serverplan_steps' and entry in {
            'collect_current_step022_blockers', 'collect_current_step023_blockers'}:
        return 'serverplan-steps-collectors.rr'
    return check.replace('_', '-')+'.rr'
