import hashlib

from cryptography.hazmat.primitives import hashes


def crypto_md5():
    return hashes.Hash(hashes.MD5())


def crypto_sha1():
    return hashes.Hash(hashes.SHA1())


def crypto_sha256():
    return hashes.Hash(hashes.SHA256())


def fingerprint(data):
    # B324's business, not B303's.
    return hashlib.md5(data).hexdigest()
