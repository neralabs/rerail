import logging

try:
    pass
except Exception:
    logging.error("failed")

import logging

try:
    pass
except Exception:
    logging.error("failed")  # noqa

import logging

try:
    pass
except Exception:
    logging.error("failed")  # noqa: TRY400

import logging

try:
    pass
except Exception:
    logging.error("failed")  # NOQA: TRY400

import logging

try:
    pass
except Exception:
    logging.error("failed")  # noqa: E501
