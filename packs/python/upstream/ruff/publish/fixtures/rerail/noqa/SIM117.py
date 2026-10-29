import contextlib
with contextlib.suppress(ValueError):
    with contextlib.suppress(KeyError):
        pass

import contextlib
with contextlib.suppress(ValueError):  # noqa
    with contextlib.suppress(KeyError):
        pass

import contextlib
with contextlib.suppress(ValueError):  # noqa: SIM117
    with contextlib.suppress(KeyError):
        pass

import contextlib
with contextlib.suppress(ValueError):  # NOQA: SIM117
    with contextlib.suppress(KeyError):
        pass

import contextlib
with contextlib.suppress(ValueError):  # noqa: E501
    with contextlib.suppress(KeyError):
        pass
