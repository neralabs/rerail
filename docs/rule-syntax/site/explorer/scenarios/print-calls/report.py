import logging

log = logging.getLogger(__name__)


def show(rows):
    for row in rows:
        print(row)


def banner():
    print("== report ==")


def quiet(rows):
    log.info("%d rows", len(rows))


class Printer:
    def print(self, text):
        return text


Printer().print("not the builtin")
