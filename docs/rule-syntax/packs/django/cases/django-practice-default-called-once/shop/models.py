import datetime
import random
import secrets
import time
import uuid
from datetime import date, timedelta
from datetime import datetime as dt
from uuid import uuid4

from django.db import models
from django.db.models import CharField, UUIDField
from django.utils import timezone
from django.utils.crypto import get_random_string
from django.utils.timezone import now


def in_a_week():
    return timezone.now() + timedelta(days=7)


class ShortUUIDField(models.CharField):
    pass


class StampField(models.DateTimeField):
    pass


class Invite(models.Model):
    id = models.UUIDField(primary_key=True, default=uuid.uuid4())  # caught
    key = UUIDField(default=uuid4())  # caught
    code = models.CharField(max_length=32, default=str(uuid.uuid4()))  # caught
    hex = CharField(max_length=32, default=uuid.uuid4().hex)  # caught
    token = models.CharField(max_length=43, default=secrets.token_urlsafe(32))  # caught
    pin = models.IntegerField(default=random.randint(1000, 9999))  # caught
    secret = models.CharField(max_length=32, default=get_random_string(32))  # caught
    expires = models.DateTimeField(default=timezone.now() + timedelta(days=7))  # caught
    expires_naive = models.DateTimeField(default=datetime.datetime.now() + datetime.timedelta(hours=1))  # caught
    stamp = models.CharField(max_length=40, default=now().isoformat())  # caught
    started = models.FloatField(
        default=time.time(),  # caught
    )
    owner = models.ForeignKey(
        "auth.User",
        on_delete=models.CASCADE,
        limit_choices_to={"date_joined__lte": date.today()},  # caught
    )

    # Callables: Django calls them for each new object.
    good_id = models.UUIDField(default=uuid.uuid4)
    good_key = models.UUIDField(default=uuid4)
    good_expires = models.DateTimeField(default=in_a_week)
    good_created = models.DateTimeField(default=timezone.now)
    good_lambda = models.CharField(max_length=32, default=lambda: get_random_string(32))
    # A deterministic value: uuid5 of a constant is the same every time.
    namespace = models.UUIDField(default=uuid.uuid5(uuid.NAMESPACE_DNS, "example.com"))
    # fields.W161 reports a bare now()/today() on date and time fields: left to Django.
    created = models.DateTimeField(default=timezone.now())
    day = models.DateField(default=date.today())
    day2 = models.DateField(default=timezone.now().date())
    created_utc = models.DateTimeField(default=datetime.datetime.utcnow())
    created_local = models.DateTimeField(default=timezone.localtime())
    # A naive now() is local time: W161 compares with UTC and misses it unless TIME_ZONE is UTC.
    created_naive = models.DateTimeField(default=datetime.datetime.now())  # caught
    created_today = models.DateTimeField(default=dt.today())  # caught
    # A project field class derived from a Django one.
    short = ShortUUIDField(max_length=32, default=uuid.uuid4().hex)  # caught
    stamp2 = StampField(default=timezone.now())
    # Not a model field.
    other = dict(default=uuid.uuid4())


def make_field():
    # Inside a def the field is built per call, but its default is still computed once for that field.
    return models.CharField(max_length=32, default=get_random_string(8))  # caught
