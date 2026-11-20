from contextlib import contextmanager, nullcontext
from functools import wraps

from celery import shared_task
from django.db import transaction

from .models import Hold


@contextmanager
def project_gate(nested):
    if nested:
        # The outer gate holds the transaction already.
        yield
        return
    with (
        transaction.atomic(),
        nullcontext(),
    ):
        yield


@contextmanager
def operation():
    with project_gate(nested=False):
        yield


def operation_method(method):
    @wraps(method)
    def wrapped(*args, **kwargs):
        with operation():
            return method(*args, **kwargs)

    return wrapped


@shared_task
def edit(pk):
    # operation() yields inside project_gate(), which yields inside transaction.atomic().
    with operation():
        return Hold.objects.select_for_update().get(pk=pk)


class Editor:
    @operation_method
    def apply(self, pk):
        # The decorator calls it inside operation().
        return Hold.objects.select_for_update().get(pk=pk)


@contextmanager
def plain_gate():
    yield


@shared_task
def edit_ungated(pk):
    with plain_gate():
        return Hold.objects.select_for_update().get(pk=pk)  # caught
