from unittest.mock import patch

from shop.alerts import alert


# shop/notify/__init__.py passes its copy on to shop/alerts.py: neither patch reaches `sms` there.
@patch("shop.notify.send_sms")  # caught
@patch("shop.notify.sms.send_sms")  # caught
def test_alert(inner, outer):
    alert("+100")


# The test file imports no module that holds a copy: nothing tells which code it runs, so it is not reported.
@patch("shop.mail.TIMEOUT", 1)
def test_no_importer_in_sight():
    alert("+100")
