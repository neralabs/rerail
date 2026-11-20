from django.db import transaction

from plates.gate import lock_plates


def rebuild(pk):
    with transaction.atomic():
        lock_plates(pk)
