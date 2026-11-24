import uuid

from django.db import models


class Invite(models.Model):
    id = models.UUIDField(primary_key=True, default=uuid.uuid4)
    email = models.EmailField(unique=True)
    token = models.CharField(max_length=64)


class Label(models.Model):
    name = models.CharField(max_length=64, unique=True)
    color = models.CharField(max_length=7)


class Issue(models.Model):
    title = models.CharField(max_length=200, unique=True)
    label = models.ForeignKey(Label, null=True, on_delete=models.SET_NULL)


class IssueSequence(models.Model):
    issue = models.ForeignKey(Issue, on_delete=models.CASCADE)
    sequence = models.IntegerField()


class Tag(models.Model):
    name = models.CharField(max_length=64, unique=True)


class Article(models.Model):
    tags = models.ManyToManyField(Tag)
