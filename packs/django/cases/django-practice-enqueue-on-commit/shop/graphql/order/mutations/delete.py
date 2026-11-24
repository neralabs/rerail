# a relative import of four dots
from django.db import transaction

from .... import tasks
from ....tasks import notify_order


def delete(order):
    with transaction.atomic():
        order.delete()
        notify_order.delay(order.pk)  # caught
        tasks.render_invoice.delay(order.pk)  # caught
