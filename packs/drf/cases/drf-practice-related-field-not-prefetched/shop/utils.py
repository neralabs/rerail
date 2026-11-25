import logging

logger = logging.getLogger(__name__)


def prefetch_related_objects(rows):
    """The project's own helper: it logs the page, it loads nothing."""
    logger.debug("page of %d rows", len(rows))
