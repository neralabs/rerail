import random
import secrets
from random import choice

ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"


def weak_token(length=16):
    return "".join(choice(ALPHABET) for _ in range(length))


def weak_pin():
    return random.randint(0, 9999)


def strong_token(length=16):
    return "".join(secrets.choice(ALPHABET) for _ in range(length))


def strong_hex():
    return secrets.token_hex(16)
