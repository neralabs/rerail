from django.db import models


class Owned(models.Model):
    owner = models.ForeignKey("auth.User", on_delete=models.CASCADE)

    class Meta:
        abstract = True

    @property
    def owner_groups(self):
        return self.owner.groups.filter(name__startswith="shop")
