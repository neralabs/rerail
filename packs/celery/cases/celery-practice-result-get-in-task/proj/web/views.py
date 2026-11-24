from proj import tasks
from proj.helpers import render_report, web_only


def report_view(request, account_id):
    tasks.price.delay(account_id).get()
    web_only(account_id)
    return render_report(account_id)
