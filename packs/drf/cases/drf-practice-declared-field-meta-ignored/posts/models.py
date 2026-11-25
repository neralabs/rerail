from django.contrib.auth.models import User
from django.db import models


class Post(models.Model):
    author = models.ForeignKey(User, on_delete=models.CASCADE)
    editor = models.ForeignKey(User, on_delete=models.SET_NULL, null=True, related_name="+")
    title = models.CharField(max_length=10)
    slug = models.SlugField()
    summary = models.TextField()
