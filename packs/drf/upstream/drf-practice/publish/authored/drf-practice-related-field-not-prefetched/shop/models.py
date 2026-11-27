from annoying.fields import AutoOneToOneField
from django.contrib.auth.models import User
from django.db import models


class Country(models.Model):
    name = models.CharField(max_length=80)


class Author(models.Model):
    name = models.CharField(max_length=120)
    country = models.ForeignKey(Country, on_delete=models.CASCADE)


class Publisher(models.Model):
    name = models.CharField(max_length=120)


class Tag(models.Model):
    name = models.CharField(max_length=40)


class Book(models.Model):
    title = models.CharField(max_length=200)
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    publisher = models.ForeignKey("Publisher", null=True, on_delete=models.SET_NULL)
    tags = models.ManyToManyField(Tag, blank=True)
    active = models.BooleanField(default=True)


class Review(models.Model):
    book = models.ForeignKey(Book, related_name="reviews", on_delete=models.CASCADE)
    text = models.TextField()


class Chapter(models.Model):
    book = models.ForeignKey("shop.Book", on_delete=models.CASCADE)
    number = models.IntegerField()


class ArticleManager(models.Manager):
    def get_queryset(self):
        return super().get_queryset().select_related("author")


class Article(models.Model):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    objects = ArticleManager()


class Theme(models.Model):
    name = models.CharField(max_length=40)


# One row per user: a OneToOneField, a third-party OneToOneField subclass, a unique ForeignKey.
class UserPreference(models.Model):
    user = models.OneToOneField(User, on_delete=models.CASCADE)
    theme = models.ForeignKey(Theme, on_delete=models.CASCADE)
    shared_with = models.ManyToManyField(User, related_name="+")


class SearchPreference(models.Model):
    user = AutoOneToOneField(User, on_delete=models.CASCADE, primary_key=True)
    theme = models.ForeignKey(Theme, on_delete=models.CASCADE)


class Profile(models.Model):
    owner = models.ForeignKey(User, unique=True, on_delete=models.CASCADE)
    theme = models.ForeignKey(Theme, on_delete=models.CASCADE)


# Many rows per user: a plain ForeignKey.
class Note(models.Model):
    user = models.ForeignKey(User, on_delete=models.CASCADE)
    theme = models.ForeignKey(Theme, on_delete=models.CASCADE)
