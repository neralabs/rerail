from django.conf import settings
from django.db import models


class Organization(models.Model):
    name = models.CharField(max_length=100)


class Shelf(models.Model):
    slug = models.SlugField(unique=True)
    owner = models.ForeignKey(settings.AUTH_USER_MODEL, on_delete=models.CASCADE)


class Book(models.Model):
    title = models.CharField(max_length=100)
    shelf = models.ForeignKey(Shelf, on_delete=models.CASCADE)
    owner = models.ForeignKey(settings.AUTH_USER_MODEL, on_delete=models.CASCADE)
    organization = models.ForeignKey(Organization, on_delete=models.CASCADE)

    def has_permission(self, user):
        return self.organization_id == user.active_organization_id
