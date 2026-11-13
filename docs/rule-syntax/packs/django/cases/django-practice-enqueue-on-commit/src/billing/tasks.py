from billing.base import AfterCommitTask
from billing.celery import app


@app.task(base=AfterCommitTask)
def charge(invoice_id):
    pass


@app.task
def refund(invoice_id):
    pass
