from django.db import models


class Author(models.Model):
    name = models.CharField(max_length=100)
    password = models.CharField(max_length=128)


class Book(models.Model):
    title = models.CharField(max_length=200)
    year = models.IntegerField()
    published = models.DateTimeField()
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
