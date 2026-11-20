import logging

log = logging.getLogger(__name__)


def start(port, host):
    log.info(f"listening on {host}:{port}")
    logging.warning(f"debug mode on {host}")
    log.info("listening on %s:%d", host, port)


def name(user):
    return f"user {user}"
