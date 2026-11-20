from functools import partial

from celery import current_app

from django.core import mail
from django.core.mail import EmailMessage, EmailMultiAlternatives, send_mail
from django.db import transaction
from django.db.transaction import atomic, on_commit
from django_q.tasks import async_task

from shop.tasks import notify_order, render_invoice
from shop.tracing import traced_atomic_transaction


def place(order):
    with transaction.atomic():
        order.save()
        notify_order.delay(order.pk)  # caught
        render_invoice.apply_async(args=[order.pk], countdown=5)  # caught
        notify_order.s(order.pk).delay()  # caught
        current_app.send_task("shop.notify", args=[order.pk])  # caught
        async_task("shop.tasks.notify", order.pk)  # caught
        send_mail("Order", "Placed", "shop@example.com", [order.email])  # caught
        mail.mail_admins("Order", "Placed")  # caught
        EmailMessage("Order", "Placed", to=[order.email]).send()  # caught
        msg = EmailMultiAlternatives("Order", "Placed", to=[order.email])
        msg.send()  # caught


@transaction.atomic
def ship(order):
    order.shipped = True
    order.save()
    notify_order.enqueue(order_id=order.pk)  # caught


@atomic(using="default")
def ship_later(order):
    notify_order.using(priority=10).enqueue(order_id=order.pk)  # caught


def wrapped(order):
    with traced_atomic_transaction():
        order.save()
        notify_order.delay(order.pk)  # caught


def after_commit(order):
    with transaction.atomic():
        order.save()
        # these run after the commit
        transaction.on_commit(partial(notify_order.delay, order.pk))
        transaction.on_commit(lambda: notify_order.delay(order.pk))
        on_commit(notify_order.s(order.pk).delay)
        notify_order.delay_on_commit(order.pk)


def outside(order):
    with transaction.atomic():
        order.save()
    notify_order.delay(order.pk)
    send_mail("Order", "Placed", "shop@example.com", [order.email])


def nested_def(order):
    with transaction.atomic():
        order.save()

        def later():
            # runs when called, not in the block
            notify_order.delay(order.pk)

        return later


def signals(order):
    with transaction.atomic():
        order.save()
        # a signal's send() is not mail
        order_placed.send(sender=None, order=order)


def transaction_aware(order):
    from shop import tasks
    from shop.tasks import transmit_invoice

    with transaction.atomic():
        order.save()
        # these task classes defer apply_async to on_commit themselves
        transmit_invoice.apply_async(args=(order.pk,))
        tasks.archive_order.delay(order.pk)
        tasks.render_invoice.delay(order.pk)  # caught


def failure_mail(order):
    with transaction.atomic():
        try:
            order.save()
        except Exception:
            # the block rolls back: an on_commit mail would never be sent
            mail.mail_admins("charge failed", f"order {order.pk}")
            raise


def failure_mail_swallowed(order):
    with transaction.atomic():
        try:
            order.save()
        except Exception:
            mail.mail_admins("charge failed", f"order {order.pk}")  # caught


def stores_task_id(order):
    with transaction.atomic():
        # the id is stored inside the block; on_commit returns nothing
        result = render_invoice.apply_async(args=(order.pk,))
        order.task_id = result.id
        order.save()
        unused = notify_order.delay(order.pk)  # caught


def not_a_task(client, order):
    with transaction.atomic():
        order.save()
        # a parameter's delay() is not known to be Celery's
        client.delay(2.0)


def eager_on_commit(order):
    with transaction.atomic():
        order.save()
        # delay() runs now, inside the block: on_commit is handed its result
        transaction.on_commit(notify_order.delay(order.pk))  # caught
