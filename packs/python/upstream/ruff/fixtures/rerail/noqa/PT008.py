import os


def test_x(mocker):
    mocker.patch.object(os, "getcwd", lambda: "x")

import os


def test_x(mocker):
    mocker.patch.object(os, "getcwd", lambda: "x")  # noqa

import os


def test_x(mocker):
    mocker.patch.object(os, "getcwd", lambda: "x")  # noqa: PT008

import os


def test_x(mocker):
    mocker.patch.object(os, "getcwd", lambda: "x")  # NOQA: PT008

import os


def test_x(mocker):
    mocker.patch.object(os, "getcwd", lambda: "x")  # noqa: E501
