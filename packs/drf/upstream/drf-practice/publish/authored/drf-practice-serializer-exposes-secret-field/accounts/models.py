from django.contrib.auth.models import AbstractUser
from django.db import models

from core.models import CredentialModel


class User(AbstractUser):
    password_changed_at = models.DateTimeField(null=True)
    has_password = models.BooleanField(default=True)
    totp_secret = models.CharField(max_length=32, blank=True)


class ApiClient(CredentialModel):
    name = models.CharField(max_length=100)
    owner = models.ForeignKey(User, on_delete=models.CASCADE)


class Session(models.Model):
    user = models.ForeignKey(User, on_delete=models.CASCADE)
    refresh_token = models.ForeignKey("oauth.RefreshToken", on_delete=models.CASCADE)
    token_count = models.PositiveIntegerField(default=0)
    started_at = models.DateTimeField(auto_now_add=True)
