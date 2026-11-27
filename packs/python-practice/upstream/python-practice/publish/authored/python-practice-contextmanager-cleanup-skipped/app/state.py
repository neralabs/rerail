import contextvars
import os
import signal
import threading
from contextlib import contextmanager as cm

current_user = contextvars.ContextVar("current_user")
_local = threading.local()


@cm
def as_user(user):
    token = current_user.set(user)
    yield user  # caught
    current_user.reset(token)


@cm
def patched_timeout(client, seconds):
    old = client.timeout
    client.timeout = seconds
    yield client  # caught
    client.timeout = old


@cm
def env(name, value):
    old = os.environ.get(name)
    os.environ[name] = value
    yield  # caught
    if old is None:
        del os.environ[name]
    else:
        os.environ[name] = old


@cm
def ignoring_sigint():
    old = signal.signal(signal.SIGINT, signal.SIG_IGN)
    yield  # caught
    signal.signal(signal.SIGINT, old)


@cm
def working_dir(path):
    old = os.getcwd()
    os.chdir(path)
    yield path  # caught
    os.chdir(old)


def run_script(code):
    import io
    import sys

    @cm
    def captured(buffer):
        old = sys.stdout
        sys.stdout = buffer
        yield buffer  # caught
        sys.stdout = old

    with captured(io.StringIO()) as out:
        exec(code)
    return out.getvalue()


def get_user():
    return getattr(_local, "user", None)


def set_user(user):
    _local.user = user


@cm
def override_user(user):
    original_user = get_user()
    set_user(user)
    yield  # caught
    set_user(original_user)


@cm
def no_autoescape(engines):
    autoescape_orig = engines["jinja2"].env.autoescape
    engines["jinja2"].env.autoescape = False
    yield  # caught
    engines["jinja2"].env.autoescape = autoescape_orig


class Pipeline:
    @cm
    def quiet(self):
        self.suppress_warnings = True
        yield  # caught
        self.suppress_warnings = False


@cm
def keep_session_age(session):
    try:
        session_expiry = session["_session_expiry"]
    except KeyError:
        yield
    else:
        yield  # caught
        session["_session_expiry"] = session_expiry
