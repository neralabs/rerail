from functools import cached_property

from celery import shared_task

from ..core.tracing import traced_transaction
from ..core.transactions import commit_on_errors
from ..models import Hold


@shared_task
def complete(pk):
    # commit_on_errors() yields inside traced_transaction(), which yields inside transaction.atomic(); the two-dot
    # import binds nothing the call graph follows, so the context manager is found by name.
    with commit_on_errors():
        return Hold.objects.select_for_update().get(pk=pk)


@traced_transaction()
def create_order(pk):
    # A context manager used as a decorator: the body runs in its transaction.
    return Hold.objects.select_for_update().get(pk=pk)


@shared_task
def checkout(pk):
    return create_order(pk)


class Reordering:
    def __init__(self, qs):
        self.qs = qs

    @cached_property
    def ordered(self):
        # Read as an attribute, never called: "nobody calls it" says nothing about its callers' transaction.
        return dict(self.qs.select_for_update().values_list("pk", "seats"))
