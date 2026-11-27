import logging
import os
import threading
from contextlib import contextmanager
from decimal import Decimal
from os import chdir as cd

from . import context
from .context import Marker, get_user, set_user

log = logging.getLogger(__name__)
_tls = threading.local()


@contextmanager
def as_user(user):
    previous = get_user()
    set_user(user)
    yield user  # caught
    set_user(previous)


@contextmanager
def as_user_qualified(user):
    previous = context.get_user()
    context.set_user(user)
    yield  # caught
    context.set_user(previous)


@contextmanager
def as_user_cleared(user):
    set_user(user)
    yield  # caught
    set_user(None)


@contextmanager
def in_dir(path):
    old = os.getcwd()
    cd(path)
    yield path  # caught
    os.chdir(old)


class Loader:
    @contextmanager
    def level(self, value):
        saved = self.get_level()
        self.set_level(value)
        yield  # caught
        self.set_level(saved)


@contextmanager
def as_request_user(request):
    original = request.user
    set_user(request.user)
    yield  # caught
    set_user(original)


@contextmanager
def labelled(key, rows):
    label = str(key)
    yield label
    log.info("done %s", str(key))


@contextmanager
def priced(items):
    total = Decimal(0)
    for item in items:
        amount = item.amount
        total += Decimal(str(amount))
    yield total
    summary = {str(amount): Decimal(str(amount)) for item in items}
    log.info("%s", summary)


@contextmanager
def announced(job):
    old = job.name
    print(job)
    yield job
    print(old)


@contextmanager
def marked(path):
    saved = path.parent
    Marker(path)
    yield path
    Marker(saved)


@contextmanager
def as_default_user(user, default):
    set_user(user)
    yield
    set_user(default)


@contextmanager
def logged(job):
    name = job.name
    log.info(name)
    yield job
    log.info(name)


@contextmanager
def reported(job):
    name = job.name
    log.info("start %s", job)
    yield job
    log.info("end %s", name)
