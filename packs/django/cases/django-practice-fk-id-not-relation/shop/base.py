from django.db import models


class Owned(models.Model):
    owner = models.ForeignKey("auth.User", on_delete=models.CASCADE)

    class Meta:
        abstract = True
