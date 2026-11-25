from django.db import models


class Document(models.Model):
    # NLP tokens of the text: not a secret.
    tokens = models.JSONField(default=list)
    title = models.CharField(max_length=10)


class Bucket(models.Model):
    # The S3 access key id identifies the account; secret_access_key is the secret.
    access_key = models.CharField(max_length=40)
    secret_access_key = models.CharField(max_length=80)


class Device(models.Model):
    key = models.CharField(max_length=40)
    pin = models.CharField(max_length=6)


class Share(models.Model):
    token = models.CharField(max_length=40)
