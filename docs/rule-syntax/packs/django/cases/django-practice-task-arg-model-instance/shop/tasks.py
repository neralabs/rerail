from celery import shared_task

from proj.celery import app


@shared_task
def ship_order(order):
    order.save()


@app.task(bind=True, max_retries=3)
def reindex_order(self, order):
    pass


@shared_task(ignore_result=True)
def email_user(user):
    pass


def send_report(order):
    pass
