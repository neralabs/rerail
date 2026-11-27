from django.db import models


class Task(models.Model):
    project_id = models.IntegerField(null=True)
    total_annotations = models.IntegerField(default=0)
