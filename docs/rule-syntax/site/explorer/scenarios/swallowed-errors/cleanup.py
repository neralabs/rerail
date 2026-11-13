import logging
import os

log = logging.getLogger(__name__)


def remove(path):
    try:
        os.remove(path)
    except Exception:
        pass


def remove_quietly(path):
    try:
        os.remove(path)
    except FileNotFoundError:
        pass


def remove_logged(path):
    try:
        os.remove(path)
    except OSError as error:
        log.warning("could not remove %s: %s", path, error)
