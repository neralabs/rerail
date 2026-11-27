from django.db import models


class Author(models.Model):
    name = models.CharField(max_length=100)
    password = models.CharField(max_length=128)


class Book(models.Model):
    title = models.CharField(max_length=200)
    year = models.IntegerField()
    published = models.DateTimeField()
    author = models.ForeignKey(Author, on_delete=models.CASCADE)


class Tag(models.Model):
    name = models.CharField(max_length=50)


class Review(models.Model):
    book = models.ForeignKey(Book, related_name="reviews", on_delete=models.CASCADE)
    text = models.TextField()


class Shelf(models.Model):
    owner = models.ForeignKey(Author, on_delete=models.CASCADE)
    books = models.ManyToManyField(Book)
