import uuid

from django import forms
from django.db import models
from django.db.models import Func, Model, expressions

from .base import Owned, TimeStamped


class Order(models.Model):
    status = models.CharField(max_length=20)
    total = models.IntegerField(default=0)
    customer = models.ForeignKey("auth.User", on_delete=models.CASCADE)

    def __init__(self, *args, **kwargs):  # caught
        super().__init__(*args, **kwargs)
        self._old_status = self.status
        self._old_total = self.total
        self._old_customer = self.customer_id


class Invoice(Model):
    number = models.CharField(max_length=20)

    def __init__(self, number, *args, **kwargs):  # caught
        super().__init__(*args, number=number, **kwargs)


class Shipment(TimeStamped):
    # A field of the base model in another module of the run.
    def __init__(self, *args, **kwargs):  # caught
        super().__init__(*args, **kwargs)
        self._was_created = self.created


class Counter(models.Model):
    hits = models.IntegerField(default=0)

    def __init__(self, *args, **kwargs):  # caught
        super().__init__(*args, **kwargs)
        self.hits += 0


class Plain(models.Model):
    name = models.CharField(max_length=20)

    # Only sets attributes that are not fields, and writes (not reads) a field: left alone.
    def __init__(self, *args, **kwargs):
        self.extra = kwargs.pop("extra", None)
        super().__init__(*args, **kwargs)
        self._cache = {}
        self.name = "x"


class SafeTracking(models.Model):
    status = models.CharField(max_length=20)

    # Reads through __dict__, which never loads a deferred field.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._old_status = self.__dict__.get("status")


class DeferredAware(models.Model):
    status = models.CharField(max_length=20)

    # Checks get_deferred_fields() before reading.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        if "status" not in self.get_deferred_fields():
            self._old_status = self.status


class KeyRead(models.Model):
    key = models.UUIDField(primary_key=True, default=uuid.uuid4)

    # The primary key is never deferred.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        if isinstance(self.key, str):
            self.key = uuid.UUID(self.key)


class LaterRead(models.Model):
    status = models.CharField(max_length=20)

    # The nested function reads the field when it is called, not in __init__.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.describe = lambda: self.status


class Concat(Func):
    # A query expression declares an output_field but is not a model.
    output_field = models.TextField()

    def __init__(self, left, right):
        super().__init__(left, right, output_field=self.output_field)


class Coalesced(expressions.Func):
    # A query expression too (its base is in django.db.models.expressions), whatever its field attribute is named.
    default_field = models.IntegerField()

    def __init__(self, *exprs):
        super().__init__(*exprs, output_field=self.default_field)


class Asset(Owned):
    # The key of a foreign key the base model in base.py declares.
    def __init__(self, *args, **kwargs):  # caught
        super().__init__(*args, **kwargs)
        self._old_owner = self.owner_id


class Profile(models.Model):
    user = models.OneToOneField("auth.User", on_delete=models.CASCADE, primary_key=True)

    # The key of a primary-key one-to-one field is the primary key, never deferred.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._old_user = self.user_id


class OrderForm(forms.ModelForm):
    status = forms.CharField()

    # Form __init__ overrides are idiomatic.
    def __init__(self, data, *args, **kwargs):
        super().__init__(data, *args, **kwargs)
        self.initial_status = self.status


class OrderManager(models.Manager):
    def __init__(self, flag, *args, **kwargs):
        super().__init__(*args, **kwargs)


class Tracked(models.Model):
    status = models.CharField(max_length=20)

    @classmethod
    def from_db(cls, db, field_names, values):
        instance = super().from_db(db, field_names, values)
        instance._old_status = instance.status
        return instance


class TupleWrite(models.Model):
    status = models.CharField(max_length=20)
    total = models.IntegerField(default=0)

    # Tuple unpacking writes both fields and reads neither.
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.status, self.total = "new", 0


class KwOnly(models.Model):
    name = models.CharField(max_length=20)

    # Model.from_db calls cls(*values): without *args every queryset load fails.
    def __init__(self, **kwargs):  # caught
        super().__init__(**kwargs)


class NoArgs(models.Model):
    name = models.CharField(max_length=20)

    def __init__(self, name="x", **kwargs):  # caught
        super().__init__(name=name, **kwargs)


class Annotated(models.Model):
    name = models.CharField(max_length=20)

    # An annotated *args still takes the row's values.
    def __init__(self, *args: object, **kwargs: object) -> None:
        super().__init__(*args, **kwargs)
        self._cache = {}
