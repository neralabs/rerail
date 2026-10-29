import logging

try:
    pass
except Exception as e:
    logging.exception(f"failed: {e}")

import logging

try:
    pass
except Exception as e:
    logging.exception(f"failed: {e}")  # noqa

import logging

try:
    pass
except Exception as e:
    logging.exception(f"failed: {e}")  # noqa: TRY401

import logging

try:
    pass
except Exception as e:
    logging.exception(f"failed: {e}")  # NOQA: TRY401

import logging

try:
    pass
except Exception as e:
    logging.exception(f"failed: {e}")  # noqa: E501
