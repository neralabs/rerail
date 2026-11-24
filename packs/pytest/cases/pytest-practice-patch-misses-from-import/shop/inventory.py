from shop.mail import audit
from shop.stock import refresh


def restock():
    audit("restock")
    refresh()
