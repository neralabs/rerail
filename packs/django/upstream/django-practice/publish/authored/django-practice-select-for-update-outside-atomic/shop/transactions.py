from contextlib import nullcontext

from django.db import transaction


def conditional_atomic(enabled):
    return transaction.atomic() if enabled else nullcontext()
