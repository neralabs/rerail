from unittest import mock
from unittest.mock import patch

import pytest

import shop.http
from shop import mail
from shop.mail import log_sent
from shop.orders import place


@mock.patch("shop.mail.send_email")  # caught
def test_place_sends_mail(send):
    place(1)
    send.assert_called_once()


def test_timeout():
    with patch("shop.mail.TIMEOUT", 1):  # caught
        place(1)


def test_started_patch():
    p = patch("shop.http.fetch")  # caught
    p.start()
    place(1)
    p.stop()


def test_client(mocker):
    mocker.patch("shop.http.Client")  # caught
    place(1)


def test_patch_object():
    with patch.object(shop.http, "fetch"):  # caught
        place(1)


def test_mocker_object(mocker):
    send = mocker.patch.object(mail, "send_email")  # caught
    place(1)
    send.assert_not_called()


def test_monkeypatch_string(monkeypatch):
    monkeypatch.setattr("shop.mail.send_email", lambda *a: None)  # caught
    place(1)


def test_monkeypatch_object(monkeypatch):
    monkeypatch.setattr(mail, "send_email", lambda *a: None)  # caught
    place(1)


# Near misses: each patch below reaches the code that reads the name.
@patch("shop.orders.send_email")
def test_right_place(send):
    place(1)


@patch("shop.mail.render_body")
def test_called_by_its_own_module(render):
    mail.confirm(1)


@patch("shop.http.post")
def test_read_through_module(post):
    pass


@patch("shop.mail.archive")
def test_imported_at_call_time(archive):
    pass


@patch("shop.mail.log_sent")
def test_only_tests_import_it(log):
    log_sent(1)


@patch("requests.get")
def test_outside_workspace(get):
    pass


@patch("shop.http.Client.get")
def test_class_attribute(get):
    place(1)


@patch("shop.mail.send_email", create=True)
def test_create(send):
    pass


@patch.dict("shop.mail.__dict__", {"TIMEOUT": 1})
def test_patch_dict():
    pass


@patch("shop.mail.missing")
def test_missing(missing):
    pass


def test_raising_false(monkeypatch):
    monkeypatch.setattr(mail, "send_email", lambda *a: None, raising=False)
    monkeypatch.delattr("shop.mail.send_email")


# Django imports a management command's module when call_command runs it, inside the patch.
@patch("shop.stock.refresh")
def test_command(refresh):
    pass


# shop/receipts.py reads Session only in a type annotation.
@patch("shop.http.Session")
def test_annotation_only(session):
    pass
