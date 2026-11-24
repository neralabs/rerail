import random
import secrets
import string
from datetime import timedelta

from django.db import models
from django.utils import timezone
from django.utils.crypto import get_random_string


def generate_secret():
    return get_random_string(length=16, allowed_chars=string.ascii_lowercase + string.digits)


def generate_code():
    # One hop: the generator is in another function of the run.
    return "C-" + generate_secret()


def pick_colour():
    return "#" + "".join(random.choices(string.hexdigits, k=6))


def new_key():
    return secrets.token_hex(16)


def default_prefs():
    return {"theme": "light"}


def default_expiry():
    return timezone.now() + timedelta(days=7)


class Customer(models.Model):
    name = models.CharField(max_length=100)


class SiteConfig(models.Model):
    name = models.CharField(max_length=100)
