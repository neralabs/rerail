# a src layout: the module is src/billing/views.py, imported as billing.views
from django.db import transaction

from billing.tasks import charge, refund


def pay(invoice):
    with transaction.atomic():
        invoice.save()
        charge.apply_async(args=(invoice.pk,))
        refund.delay(invoice.pk)  # caught
