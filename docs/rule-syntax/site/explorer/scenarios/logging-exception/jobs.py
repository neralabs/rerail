import logging

log = logging.getLogger(__name__)


def run(job):
    try:
        job()
    except Exception:
        log.error("job failed")


def run_ok(job):
    try:
        job()
    except Exception:
        log.exception("job failed")


def report():
    log.error("not inside an except clause")
