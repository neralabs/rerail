from django.db import models


class Author(models.Model):
    name = models.CharField(max_length=100, unique=True)


class Book(models.Model):
    author = models.ForeignKey(Author, on_delete=models.CASCADE)
    title = models.CharField(max_length=200)

    class Meta:
        unique_together = [("author", "title")]

    def clean(self):
        if self.title == "x":
            raise ValueError("no")
