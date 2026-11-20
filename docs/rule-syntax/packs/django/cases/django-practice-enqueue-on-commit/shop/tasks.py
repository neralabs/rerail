from shop.base import ProfiledTransactionAwareTask, TransactionAwareTask
from shop.celery import app


@app.task
def notify_order(order_id):
    pass


@app.task(bind=True)
def render_invoice(self, order_id):
    pass


@app.task(base=TransactionAwareTask)
def transmit_invoice(invoice_id):
    pass


@app.task(base=ProfiledTransactionAwareTask, acks_late=True)
def archive_order(order_id):
    pass
