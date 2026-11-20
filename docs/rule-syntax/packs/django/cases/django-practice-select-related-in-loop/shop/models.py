from django.db import models
from django.utils.functional import cached_property

from shop.base import Owned


class Author(models.Model):
    name = models.CharField(max_length=100)


class Tag(models.Model):
    name = models.CharField(max_length=100)


class Book(Owned):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    tags = models.ManyToManyField(Tag)
    title = models.CharField(max_length=100)
    data = models.JSONField(default=dict)

    @property
    def active_tags(self):
        return self.tags.filter(name__startswith="a")

    @cached_property
    def recent_reviews(self):
        return self.review_set.order_by("-pk")[:10]

    def tag_names(self):
        return [t.name for t in self.tags.all()]


class Review(models.Model):
    book = models.ForeignKey(Book, on_delete=models.CASCADE)
