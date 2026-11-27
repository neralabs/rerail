import base64
import hmac
import pickle

import cloudpickle
import dill
import jsonpickle
from django.core.cache import cache
from itsdangerous import URLSafeSerializer

SECRET = b"server-side key"
serializer = URLSafeSerializer("server-side key")


def decode(blob):
    return pickle.loads(blob)  # caught


def decode_b64(text):
    raw = base64.b64decode(text)
    return cloudpickle.loads(raw)  # caught


def decode_other(text):
    return dill.loads(text), jsonpickle.decode(text)  # caught


def from_cache(key):
    return pickle.loads(cache.get(key))


def decode_signed(token):
    return serializer.loads(token)


def decode_verified(blob, mac):
    expected = hmac.new(SECRET, blob, "sha256").hexdigest()
    if not hmac.compare_digest(expected, mac):
        raise ValueError("bad signature")
    return pickle.loads(blob)
