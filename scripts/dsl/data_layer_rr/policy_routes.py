"""Static API-to-policy routing; no expected result influences the selected program."""


def policy_filename(check, entry):
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
