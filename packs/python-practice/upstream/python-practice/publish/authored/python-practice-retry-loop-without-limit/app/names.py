import secrets
import time

from sqlalchemy.exc import IntegrityError


class EntityExistsError(Exception):
    pass


# Not a retry: each turn tries a new free name.
def create_connector(store, base):
    name = base
    while True:
        try:
            connector = store.create(name=name)
            break
        except EntityExistsError:
            name = f"{base}-{secrets.token_hex(2)}"
            continue
    return connector


def create_household(repo, data):
    original = data["name"]
    while True:
        try:
            return repo.create(data)
        except IntegrityError:
            repo.session.rollback()
            data["name"] = f"{original} ({secrets.randbelow(1000)})"


# A retry: the same row is sent again, and a lasting conflict loops forever.
def insert_row(repo, row):
    while True:  # caught
        try:
            return repo.insert(row)
        except IntegrityError:
            repo.session.rollback()
            time.sleep(1)
