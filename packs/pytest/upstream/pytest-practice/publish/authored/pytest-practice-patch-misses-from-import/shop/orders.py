from typing import TYPE_CHECKING

from shop.mail import TIMEOUT, send_email

from .http import Client, fetch

if TYPE_CHECKING:
    from shop.mail import log_sent


def place(order):
    send_email("a@example.com", str(order))
    fetch("https://example.com", timeout=TIMEOUT)
    return Client().get("/orders")
