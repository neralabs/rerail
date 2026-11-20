from django.db import models


class TimeStamped(models.Model):
    created = models.DateTimeField(auto_now_add=True)

    class Meta:
        abstract = True


class Owned(models.Model):
    owner = models.ForeignKey("auth.User", on_delete=models.CASCADE)

    class Meta:
        abstract = True
