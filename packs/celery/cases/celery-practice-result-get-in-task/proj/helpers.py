from proj import tasks


def render_report(account_id):
    rows = load_rows(account_id)
    return rows


def load_rows(account_id):
    result = tasks.price.delay(account_id)
    return result.get(timeout=30)  # caught


def collect_totals(account_ids):
    return [tasks.total.delay([i]).get() for i in account_ids]  # caught


def web_only(account_id):
    # Called from a view only.
    return tasks.price.delay(account_id).get()


def joined_rows(account_id):
    # Reached only from inside `with allow_join_result():`.
    return tasks.price.delay(account_id).get()
