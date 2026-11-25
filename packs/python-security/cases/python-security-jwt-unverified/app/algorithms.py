import jwt
from jose import jwt as jose_jwt
from jwt.algorithms import get_default_algorithms

WEAK = ["HS256", "none"]
MIXED = ("RS256", "HS512")


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


def decode_mixed(token, public_key):
    return jwt.decode(token, public_key, algorithms=["HS256", "RS256"])  # caught


def decode_mixed_constant(token, key):
    return jwt.decode(token, key, algorithms=MIXED)  # caught


def decode_jose_mixed(token, key):
    return jose_jwt.decode(token, key, algorithms=["ES256", "HS256"])  # caught


def decode_one_family(token, public_key):
    return jwt.decode(token, public_key, algorithms=["RS256", "ES256", "PS256"])


def decode_hmac_family(token, secret):
    return jwt.decode(token, secret, algorithms=("HS256", "HS512"))
