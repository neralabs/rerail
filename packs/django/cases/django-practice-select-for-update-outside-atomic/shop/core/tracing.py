from contextlib import contextmanager

from django.db import transaction


@contextmanager
def traced_transaction():
    with transaction.atomic():
        yield
