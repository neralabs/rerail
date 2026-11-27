from celery import shared_task
from django.db import connection, transaction
from django.db.models.signals import post_save

from .models import Hold
from .transactions import conditional_atomic


def dead_helper(pk):
    # A plain function nothing calls: dead code, or a helper whose callers (outside the project) hold the transaction.
    return Hold.objects.select_for_update().get(pk=pk)


@shared_task
def wrapped(pk):
    # A wrapper named like atomic opens the transaction.
    with conditional_atomic(enabled=True):
        return Hold.objects.select_for_update().get(pk=pk)


@shared_task
def guarded(pk):
    # Exits early unless a transaction is open.
    if not connection.in_atomic_block:
        raise RuntimeError("call inside a transaction")
    return Hold.objects.select_for_update().get(pk=pk)


@shared_task
def purge_in_batches():
    # Built first, evaluated inside the transaction.
    queryset = Hold.objects.select_for_update(skip_locked=True)
    while True:
        with transaction.atomic():
            pks = list(queryset.values_list("pk", flat=True)[:100])
            if not pks:
                return
            Hold.objects.filter(pk__in=pks).delete()


def run_import(fn, pk):
    with transaction.atomic():
        return fn(pk)


def import_rows(pk):
    # Handed to run_import, which calls it inside a transaction.
    return Hold.objects.select_for_update().get(pk=pk)


@shared_task
def importer(pk):
    return run_import(import_rows, pk)


def release_on_save(sender, instance, **kwargs):
    # Handed to a framework (a signal), which calls it after save(), with no transaction of its own.
    Hold.objects.select_for_update().filter(pk=instance.pk).update(seats=0)  # caught


post_save.connect(release_on_save, sender=Hold)
