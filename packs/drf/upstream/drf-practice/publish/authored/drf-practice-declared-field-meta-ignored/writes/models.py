from django.contrib.auth.models import User
from django.db import models


class Bookmark(models.Model):
    url = models.URLField()
    title = models.CharField(max_length=200)
    owner = models.ForeignKey(User, on_delete=models.CASCADE)


class Article(models.Model):
    title = models.CharField(max_length=200)
    slug = models.SlugField()
    status = models.CharField(max_length=20)
    owner = models.ForeignKey(User, on_delete=models.CASCADE, related_name="+")
    editor = models.ForeignKey(User, on_delete=models.SET_NULL, null=True, related_name="+")
    reviewer = models.ForeignKey(User, on_delete=models.SET_NULL, null=True, related_name="+")
    creation_user = models.ForeignKey(User, on_delete=models.SET_NULL, null=True, related_name="+")
    published_by = models.ForeignKey(User, on_delete=models.SET_NULL, null=True, related_name="+")
