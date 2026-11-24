from celery import Task, shared_task
from django.db import transaction
from django.core.mail import send_mail


@shared_task
def notify_order(order_id):
    send_mail("Order", "Shipped", "shop@example.com", ["a@example.com"])


def notify_later(order):
    notify_order.delay(order.pk)  # caught


def notify_after_commit(order):
    notify_order.delay(order.pk)


def nightly_digest():
    notify_order.delay(0)


class TransactionAwareTask(Task):
    def apply_async(self, *args, **kwargs):
        transaction.on_commit(lambda: super(TransactionAwareTask, self).apply_async(*args, **kwargs))


@shared_task(base=TransactionAwareTask)
def notify_when_committed(order_id):
    pass
