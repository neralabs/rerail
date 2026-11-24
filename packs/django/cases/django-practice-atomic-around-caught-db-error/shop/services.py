from django.db import IntegrityError, DatabaseError, transaction
from django.db import utils as db_utils
from django.db.transaction import atomic
from django.core.cache import cache, caches
from django.core.exceptions import ValidationError

from shop.models import Order, Tag


def add_tag(order, name):
    with transaction.atomic():
        order.save()
        try:
            Tag.objects.create(order=order, name=name)
        except IntegrityError:  # caught
            pass
        order.refresh_from_db()


@transaction.atomic
def rename(order, name):
    try:
        order.name = name
        order.save(update_fields=["name"])
    except (IntegrityError, ValueError):  # caught
        order.name = ""


@atomic(using="default")
def recount(order):
    try:
        Order.objects.filter(pk=order.pk).update(count=1)
    except db_utils.OperationalError:  # caught
        return None
    try:
        Order.objects.filter(pk=order.pk).exists()
    except DatabaseError as exc:  # caught
        log(exc)


def raw_insert(cursor, row):
    with atomic() as sid:
        try:
            cursor.execute("INSERT INTO t VALUES (%s)", [row])
        except IntegrityError:  # caught
            pass


def savepoint_inside(order, name):
    with transaction.atomic():
        try:
            # the inner atomic is the savepoint the docs ask for
            with transaction.atomic():
                Tag.objects.create(order=order, name=name)
        except IntegrityError:
            pass


def reraises(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError as exc:
            # the error leaves the atomic block, which rolls back
            raise ValidationError("duplicate") from exc


def manual_savepoint(order):
    with transaction.atomic():
        sid = transaction.savepoint()
        try:
            order.save()
        except IntegrityError:
            transaction.savepoint_rollback(sid)


def marks_rollback(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError:
            transaction.set_rollback(True)


def get_or_create_guards_itself(order, name):
    with transaction.atomic():
        try:
            Tag.objects.get_or_create(order=order, name=name)
        except IntegrityError:
            pass


def outside_atomic(order):
    # the error is caught around the transaction, as the docs show
    try:
        with transaction.atomic():
            order.save()
    except IntegrityError:
        pass


def other_error(order):
    with transaction.atomic():
        try:
            order.save()
        except ValueError:
            pass


def nested_def(order):
    with transaction.atomic():
        def later():
            # runs whenever it is called, not in this block
            try:
                order.save()
            except IntegrityError:
                pass
        return later


def no_db_call(data):
    with transaction.atomic():
        try:
            value = parse(data)
        except IntegrityError:
            value = None
    return value


def reraises_on_every_path(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError as exc:
            # both branches raise, so the block still rolls back
            if "unique" in str(exc):
                raise ValidationError("duplicate") from exc
            else:
                raise


def reraises_on_one_path(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError as exc:  # caught
            if "unique" in str(exc):
                raise ValidationError("duplicate") from exc


def update_or_create_guards_itself(order):
    with transaction.atomic():
        try:
            # select_for_update() only builds the queryset; update_or_create runs in its own atomic block
            Order.objects.select_for_update().update_or_create(pk=order.pk, defaults={"name": "x"})
        except IntegrityError:
            pass


def dict_update_only(names, stats):
    with transaction.atomic():
        for name in names:
            try:
                # get_or_create guards itself; `stats.update` is a dict, not a query
                tag, created = Tag.objects.get_or_create(name=name)
                stats.update(created=created)
            except IntegrityError:
                continue


def cache_delete_only(name):
    with transaction.atomic():
        try:
            # cache.delete is no database write
            Tag.objects.get_or_create(name=name)
            cache.delete("tags")
            caches["default"].delete("tags")
        except IntegrityError:
            pass


def queryset_update(order):
    with transaction.atomic():
        try:
            order.lines.filter(qty=0).update(qty=1)
        except IntegrityError:  # caught
            pass


def elif_reraises(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError as exc:
            # all three branches raise
            if "unique" in str(exc):
                raise ValidationError("duplicate") from exc
            elif "check" in str(exc):
                raise ValidationError("invalid") from exc
            else:
                raise


def elif_falls_through(order):
    with transaction.atomic():
        try:
            order.save()
        except IntegrityError as exc:  # caught
            if "unique" in str(exc):
                raise ValidationError("duplicate") from exc
            elif "check" in str(exc):
                log(exc)
            else:
                raise


def savepoint_false(order):
    with transaction.atomic():
        try:
            # savepoint=False makes no savepoint: the outer transaction is still broken after the error
            with transaction.atomic(savepoint=False):
                order.save()
        except IntegrityError:  # caught
            pass
        order.refresh_from_db()


def read_only_try(order):
    with transaction.atomic():
        try:
            n = Tag.objects.filter(name="x").count()
        except IntegrityError:  # caught
            n = 0
        return n
