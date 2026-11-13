from django.db import transaction

from shop.tasks import notify_order


def test_place(order):
    with transaction.atomic():
        notify_order.delay(order.pk)
