from contextlib import contextmanager

from celery import shared_task

from django.db import transaction

from .models import Hold


@contextmanager
def locked_holds(pks):
    # Enter right after transaction.atomic(); save_all does, sloppy_save does not.
    list(Hold.objects.filter(pk__in=pks).select_for_update().values_list("pk", flat=True))  # caught
    yield


@contextmanager
def locked_safe(pks):
    list(Hold.objects.filter(pk__in=pks).select_for_update().values_list("pk", flat=True))
    yield


def save_all(pks):
    with transaction.atomic(), locked_holds(pks), locked_safe(pks):
        Hold.objects.filter(pk__in=pks).update(seats=0)


@shared_task
def sloppy_save(pks):
    with locked_holds(pks):
        Hold.objects.filter(pk__in=pks).update(seats=0)
