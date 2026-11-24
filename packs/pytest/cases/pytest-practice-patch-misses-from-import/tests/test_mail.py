from unittest.mock import patch

from shop import mail


# The test runs shop.mail's own code and checks it never sends: the patch is where that code would look. The file
# imports no module that holds a copy, so it is not reported.
@patch("shop.mail.send_email")
def test_confirm_sends_nothing(send):
    mail.confirm(1)
    send.assert_not_called()
