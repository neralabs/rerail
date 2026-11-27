from django.db import models


class Sample(models.Model):
    parent = models.ForeignKey("self", null=True, on_delete=models.CASCADE)

    def parent_key(self):
        return self.parent.pk
