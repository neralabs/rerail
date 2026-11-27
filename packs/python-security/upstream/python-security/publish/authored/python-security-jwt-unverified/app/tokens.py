import json

import jwt


def peek_claims(token: str) -> dict:
    """Read a token's claims without checking it, to route it."""
    return jwt.decode(token, options={"verify_signature": False})


def fetch_token_header(token):
    token = token.encode("utf-8")
    signing_input, _ = token.rsplit(b".", 1)
    header_segment, _ = signing_input.split(b".", 1)
    return json.loads(jwt.utils.base64url_decode(header_segment).decode("utf-8"))


def decode_unverified(token):
    return jwt.decode(token, verify=False)  # caught
