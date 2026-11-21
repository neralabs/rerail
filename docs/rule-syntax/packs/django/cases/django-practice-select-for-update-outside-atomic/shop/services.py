from django.db import transaction

from .models import Hold


def lock_hold(pk):
    # bare_view calls this outside a transaction (atomic_view's call is covered, bare_view's is not).
    return Hold.objects.select_for_update().get(pk=pk)  # caught


def lock_safe(pk):
    # Every caller calls this inside transaction.atomic().
    return Hold.objects.select_for_update().get(pk=pk)


@transaction.atomic
def reserve(pk):
    hold = Hold.objects.select_for_update().get(pk=pk)
    hold.seats -= 1
    hold.save()


def inner(pk):
    return Hold.objects.select_for_update(nowait=True).filter(pk=pk).first()


@transaction.atomic
def outer(pk):
    return inner(pk)


def deep(pk):
    return list(Hold.objects.filter(pk=pk).select_for_update())  # caught


def mid(pk):
    return deep(pk)


def deep_safe(pk):
    return list(Hold.objects.filter(pk=pk).select_for_update())


def mid_safe(pk):
    return deep_safe(pk)


def locked_seats(pk):
    with transaction.atomic():
        hold = Hold.objects.select_for_update().get(pk=pk)
        return hold.seats


def by_unknown(pk):
    # Its only caller's target is unknown (a parameter's method), so the rule cannot tell.
    return Hold.objects.select_for_update().get(pk=pk)


def run_through(service, pk):
    return service.by_unknown(pk)


def after_commit(pk):
    with transaction.atomic():
        hold = Hold.objects.select_for_update().get(pk=pk)

        def notify():
            # on_commit runs this after the transaction ends.
            Hold.objects.select_for_update().filter(pk=hold.pk).update(seats=0)  # caught

        transaction.on_commit(notify)


class TargetedAction:
    is_atomic = True

    def dispatch(self, request, pk):
        if self.is_atomic:
            with transaction.atomic():
                return self.run(pk)
        return self.run(pk)

    def run(self, pk):
        queryset = Hold.objects.all()
        if self.is_atomic:
            # Locked only on the branch dispatch wraps in a transaction.
            queryset = queryset.select_for_update()
        return queryset.get(pk=pk)

    def run_unguarded(self, pk):
        return Hold.objects.select_for_update().get(pk=pk) if self.is_atomic else None


def get_hold(pk, lock=False):
    # The caller asks for the lock; read_hold does not, so it needs no transaction.
    queryset = Hold.objects.all()
    if lock:
        queryset = queryset.select_for_update()
    return queryset.get(pk=pk)


def get_group(pk, for_read):
    if for_read:
        return Hold.objects.get(pk=pk)
    else:
        return Hold.objects.select_for_update().get(pk=pk)


def read_hold(pk):
    return get_hold(pk), get_group(pk, for_read=True)
