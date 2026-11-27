from unittest.mock import patch

from shop.inventory import restock


# The same test also patches the copy shop/inventory.py reads.
@patch("shop.inventory.audit")
@patch("shop.mail.audit")
def test_both_places(audit, inventory_audit):
    restock()


@patch("shop.mail.audit")  # caught
def test_one_place(audit):
    restock()
