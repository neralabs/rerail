import random
import secrets
import string
from datetime import timedelta

from django.contrib.sites.models import Site
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


def pick_tagline():
    return random.choice(["Hello", "Welcome", "Hi there"])


def colour_label():
    # Two hops to random.choices: still only the random module's shared generator.
    return "colour " + pick_colour()


def system_pin():
    # SystemRandom draws from os.urandom: a secret, not a cosmetic pick.
    return "".join(random.SystemRandom().choice(string.digits) for _ in range(6))


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


class StoreSettings(models.Model):
    # One row per site, read through Site.objects.get_current().store_settings.
    site = models.OneToOneField(Site, related_name="store_settings", on_delete=models.CASCADE)


class Profile(models.Model):
    # One row per user: a one-to-one read through request.user.profile says nothing about the row count.
    user = models.OneToOneField("auth.User", related_name="profile", on_delete=models.CASCADE)
