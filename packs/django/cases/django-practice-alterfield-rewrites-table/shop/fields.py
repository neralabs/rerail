from django.db import models


class MoneyField(models.DecimalField):
    """A project field class: its column is a DecimalField's numeric."""


class LabelField(models.CharField):
    def __init__(self, *args, **kwargs):
        kwargs.setdefault("max_length", 30)
        super().__init__(*args, **kwargs)


class GeoPointField(models.Field):
    def db_type(self, connection):
        return "point"
