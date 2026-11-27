import jwt


def decode_for_assertions(token):
    return jwt.decode(token, options={"verify_signature": False})


def test_token_has_subject(make_token):
    claims = decode_for_assertions(make_token(sub="42"))
    assert claims["sub"] == "42"
