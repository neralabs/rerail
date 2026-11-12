from django.db import models
from django.db.models import UniqueConstraint, Q

from .base import Named, Timestamped


class Author(models.Model):
    email = models.EmailField(unique=True)
    name = models.CharField(max_length=100)
    country = models.CharField(max_length=2)


class Profile(models.Model):
    author = models.OneToOneField(Author, on_delete=models.CASCADE)
    bio = models.TextField()


class Tag(Timestamped):
    label = models.CharField(max_length=30)
    slug = models.SlugField()


class Shelf(Named):
    color = models.CharField(max_length=10)


class Book(models.Model):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    title = models.CharField(max_length=200)
    isbn = models.CharField(max_length=13)
    year = models.IntegerField()
    edition = models.IntegerField()

    class Meta:
        constraints = [
            UniqueConstraint(fields=["author", "title"], name="uniq_author_title"),
            models.UniqueConstraint(fields=["isbn"], condition=Q(year__gt=2000), name="uniq_isbn_recent"),
        ]
        unique_together = [["year", "edition"]]


class Visit(models.Model):
    path = models.CharField(max_length=200)
    day = models.DateField()

    class Meta:
        constraints = [models.UniqueConstraint(models.functions.Lower("path"), name="uniq_lower_path")]


class ConfigManager(models.Manager):
    def get_or_create(self, **kwargs):
        data = kwargs.pop("data")
        return super().get_or_create(data_hash=hash(data), defaults={"data": data}, **kwargs)


class ConfigQuerySet(models.QuerySet):
    def update_or_create(self, defaults=None, **kwargs):
        return super().update_or_create(defaults=defaults, **kwargs)


class Config(models.Model):
    data = models.JSONField()
    data_hash = models.CharField(max_length=64, unique=True)
    objects = ConfigManager()


class Loan(models.Model):
    book = models.ForeignKey(Book, on_delete=models.CASCADE)
    key = models.CharField(max_length=64)

    class Meta:
        constraints = [models.UniqueConstraint(fields=["book_id", "key"], name="uniq_loan_key")]


class Setting(models.Model):
    key = models.CharField(max_length=64)
    objects = ConfigQuerySet.as_manager()


class Legacy(models.Model):
    ref = models.CharField(max_length=10)

    class Meta:
        managed = False
        db_table = "legacy"


class Code(models.Model):
    value = models.CharField(max_length=10, primary_key=True)
    kind = models.CharField(max_length=10)

    @classmethod
    def ensure(cls, kind):
        return cls.objects.get_or_create(kind=kind)  # caught
