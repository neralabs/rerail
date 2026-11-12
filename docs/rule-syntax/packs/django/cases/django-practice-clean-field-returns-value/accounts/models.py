from django.contrib.auth.models import User  # noqa: F401
from django.db import models


class Profile(models.Model):
    bio = models.TextField()
    website = models.URLField()
