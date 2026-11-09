"""Static API-to-policy routing; no expected result influences the selected program."""


def policy_filename(check, entry):
    if check == 'serverplan_steps' and entry in {
            'collect_current_step022_blockers', 'collect_current_step023_blockers'}:
        return 'serverplan-steps-collectors.rr'
    return check.replace('_', '-')+'.rr'
