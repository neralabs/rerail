from django.db import models
from django.db.models import OneToOneField

from shop.base import Owned


class Author(models.Model):
    name = models.CharField(max_length=100)


class Book(Owned):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    cover = OneToOneField("shop.Cover", null=True, on_delete=models.SET_NULL)
    isbn_author = models.ForeignKey(Author, to_field="name", on_delete=models.CASCADE, related_name="+")
    title = models.CharField(max_length=100)

    def author_key(self):
        return self.author.id  # caught

    @property
    def cover_key(self):
        return self.cover.pk  # caught

    def owner_key(self):
        # owner is declared on the abstract base in another module.
        return self.owner.pk  # caught

    def keys(self):
        def inner():
            return self.author.pk  # caught
        return inner(), self.author_id

    def left_alone(self):
        # The related object's other fields need the row.
        name = self.author.name
        # to_field: isbn_author_id holds Author.name, not the key.
        key = self.isbn_author.pk
        # Not a relation.
        t = self.title.upper()
        return name, key, t

    def save(self, *args, **kwargs):
        # Is the related object saved yet? author_id stays None until this model is saved.
        if not self.author.pk:
            self.author.save()
        if self.cover is not None and self.cover.pk is None:
            self.cover = None
        assert self.owner.id, "owner must be saved"
        saved = bool(self.cover.pk)
        covers = [c for c in (self.cover,) if self.cover.pk]
        super().save(*args, **kwargs)


class Cover(models.Model):
    image = models.CharField(max_length=100)


class BookInfo:
    # Not a model: a wrapper that only shares the attribute name.
    def __init__(self, author):
        self.author = author

    def key(self):
        return self.author.id


def outside(book):
    # The object's class is not known outside the model.
    return book.author.id
