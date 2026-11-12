from django.db import models


class Timestamped(models.Model):
    code = models.CharField(max_length=20, unique=True)
    created = models.DateTimeField(auto_now_add=True)

    class Meta:
        abstract = True


class Named(models.Model):
    name = models.CharField(max_length=50)
    owner = models.ForeignKey("auth.User", on_delete=models.CASCADE)

    class Meta:
        abstract = True
        unique_together = ("name", "owner")
