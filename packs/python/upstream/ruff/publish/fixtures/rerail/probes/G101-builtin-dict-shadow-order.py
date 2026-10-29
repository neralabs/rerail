import logging
logging.info("x", extra=dict(name=1))
dict = MyDict
logging.info("x", extra=dict(name=2))


def f():
    logging.info("x", extra=dict(msg=1))
