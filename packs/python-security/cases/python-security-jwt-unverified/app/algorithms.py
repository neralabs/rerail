import jwt
from jwt.algorithms import get_default_algorithms

WEAK = ["HS256", "none"]


def decode_any_algorithm(token, public_key):
    return jwt.decode(token, public_key, algorithms=get_default_algorithms())  # caught


def decode_any_algorithm_keys(token, public_key):
    return jwt.decode(token, public_key, algorithms=list(jwt.algorithms.get_default_algorithms()))  # caught


def decode_allowing_none(token, key):
    return jwt.decode(token, key, algorithms=["HS256", "none"])  # caught


def decode_allowing_none_constant(token, key):
    return jwt.decode(token, key, algorithms=WEAK)  # caught


def decode_pyjwt2_default(token, key):
    # PyJWT 2 raises without `algorithms`: not a bypass.
    return jwt.decode(token, key)


def decode_exp_unchecked(token, key):
    return jwt.decode(token, key, algorithms=["HS256"], options={"verify_exp": False})


def decode_pinned_setting(token, settings):
    return jwt.decode(token, settings.JWT_KEY, algorithms=[settings.JWT_ALGORITHM])
