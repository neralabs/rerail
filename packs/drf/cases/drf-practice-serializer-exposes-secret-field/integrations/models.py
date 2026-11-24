from django.db import models

from core.models import CredentialModel


class EncryptedTextField(models.TextField):
    pass


class Webhook(models.Model):
    url = models.URLField()
    secret_santa = models.BooleanField(default=False)
    signing_secret = models.CharField(max_length=64)
    private_key = models.TextField()
    commit_hash = models.CharField(max_length=40)
    password_hash = models.CharField(max_length=128)
    public_key_fingerprint = models.CharField(max_length=64)


class MailAccount(models.Model):
    imap_server = models.CharField(max_length=200)
    username = models.CharField(max_length=200)
    password = models.CharField(max_length=200)
    oauth_credentials = EncryptedTextField(blank=True)


class Provider(CredentialModel):
    name = models.CharField(max_length=100)
    llm_api_key = models.CharField(max_length=200)
    client_secret = models.CharField(max_length=200)
