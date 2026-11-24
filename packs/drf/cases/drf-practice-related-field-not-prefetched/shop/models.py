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
