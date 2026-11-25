from unittest import TestCase, mock
from unittest.mock import PropertyMock, patch

import pytest

from shop import mail
from shop.mail import Mailer


# Two string patches, parameters named top-down: mock_send holds the render mock.
@patch("shop.mail.send")
@patch("shop.mail.render")
def test_two_swapped(mock_send, mock_render):  # caught
    mock_render.return_value = "<p>hi</p>"
    mail.notify("a@example.com")
    mock_send.assert_called_once()


# The safe idiom: bottom-up names.
@patch("shop.mail.send")
@patch("shop.mail.render")
def test_two_bottom_up(mock_render, mock_send):
    mail.notify("a@example.com")
    mock_send.assert_called_once()


# Three patches over a multi-line signature: the first mismatched parameter is reported; the middle one is right.
@mock.patch("shop.mail.connect")
@mock.patch("shop.mail.render")
@mock.patch("shop.mail.send")
def test_three_swapped(
    mock_connect,  # caught
    mock_render,
    mock_send,
):
    mail.notify("a@example.com")


# A mark between the patches is skipped; a fixture after the mocks is not a mock.
@patch("shop.mail.send")
@pytest.mark.django_db
@patch("shop.mail.render")
def test_mark_between(send_mock, render_mock, settings):  # caught
    mail.notify("a@example.com")


# A patch with a replacement (`new`) passes no mock: only the two others count.
@patch("shop.mail.send")
@patch("shop.mail.render", new=lambda *a: "")
@patch("shop.mail.connect")
def test_new_in_middle(mock_send, mock_connect):  # caught
    mail.notify("a@example.com")


@patch("shop.mail.send")
@patch("shop.mail.render", lambda *a: "")
@patch("shop.mail.connect")
def test_new_in_middle_ok(mock_connect, mock_send):
    mail.notify("a@example.com")


# patch.dict and patch.multiple pass nothing to the def (patch.multiple with DEFAULT is not modelled).
@patch("shop.mail.send")
@patch.dict("os.environ", {"MAIL": "1"})
@patch("shop.mail.render")
def test_dict_between(mock_send, mock_render):  # caught
    mail.notify("a@example.com")


@patch("shop.mail.send")
@patch.dict("os.environ", {"MAIL": "1"})
@patch("shop.mail.render")
def test_dict_between_ok(mock_render, mock_send):
    mail.notify("a@example.com")


# Names in another case and as a suffix; new_callable and target= still pass a mock.
@patch(target="os.path.exists")
@patch("shop.mail.Mailer.host", new_callable=PropertyMock)
def test_camel_case(mockExists, hostMock):  # caught
    assert mail.ready()


# Names that follow neither order say nothing about intent.
@patch("shop.mail.send")
@patch("shop.mail.render")
def test_neutral_names(m1, m2):
    mail.notify("a@example.com")


# Two targets with the same last name: ambiguous.
@patch("shop.mail.send")
@patch("shop.sms.send")
def test_same_last_name(mock_send, mock_send_sms):
    mail.notify("a@example.com")


# A parameter whose name holds both of its candidate targets: ambiguous.
@patch("shop.mail.user")
@patch("shop.mail.user_email")
def test_name_holds_both(mock_user, mock_user_email):
    mail.notify("a@example.com")


# One injecting patch: nothing to swap.
@patch("shop.mail.send")
def test_single(mock_send):
    mail.notify("a@example.com")


# Fewer positional parameters than mocks (they arrive in *args): not read.
@patch("shop.mail.send")
@patch("shop.mail.render")
def test_varargs(*mocks):
    mail.notify("a@example.com")


# Not a test: a helper decorated the same way is left alone.
@patch("shop.mail.send")
@patch("shop.mail.render")
def helper(mock_send, mock_render):
    return mock_send, mock_render


@patch.object(Mailer, "connect")
@patch.object(Mailer, "_send")
async def test_async_object(mock_connect, mock_send):  # caught
    await Mailer().deliver()


# camelCase targets: names compared without underscores, so `mock_assert_true` is the mock of `assertTrue`.
class TestCamelCase(TestCase):
    @mock.patch.object(TestCase, "assertTrue")
    @mock.patch("shop.mail.redis")
    def test_swapped(self, mock_assert_true, mock_redis):  # caught
        mail.notify("a@example.com")

    @mock.patch("shop.mail.EmailClient")
    @mock.patch("shop.mail.render")
    def test_class_target(self, mock_render, mock_email_client):
        mail.notify("a@example.com")

    @mock.patch("shop.mail.EmailClient")
    @mock.patch("shop.mail.render")
    def test_class_target_swapped(self, mock_email_client, mock_render):  # caught
        mail.notify("a@example.com")
