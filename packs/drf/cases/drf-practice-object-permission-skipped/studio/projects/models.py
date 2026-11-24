from django.db import models


class Project(models.Model):
    title = models.CharField(max_length=100)
    organization = models.ForeignKey("library.Organization", on_delete=models.CASCADE)

    def has_permission(self, user):
        return self.organization_id == user.active_organization_id
