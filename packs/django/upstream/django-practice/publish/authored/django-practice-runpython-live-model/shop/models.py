from django.db import models


class Author(models.Model):
    name = models.CharField(max_length=100)
    book_count = models.IntegerField(default=0)


class Book(models.Model):
    STATUS_DONE = "done"

    title = models.CharField(max_length=200)
    slug = models.SlugField(blank=True)
    status = models.CharField(max_length=10, default="")
    author = models.ForeignKey(Author, on_delete=models.CASCADE)


class Shelf(models.Model):
    name = models.CharField(max_length=50)
