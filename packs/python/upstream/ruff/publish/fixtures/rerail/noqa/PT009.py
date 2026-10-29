import unittest


class T(unittest.TestCase):
    def test_x(self):
        self.assertEqual(1, 1)

import unittest


class T(unittest.TestCase):
    def test_x(self):
        self.assertEqual(1, 1)  # noqa

import unittest


class T(unittest.TestCase):
    def test_x(self):
        self.assertEqual(1, 1)  # noqa: PT009

import unittest


class T(unittest.TestCase):
    def test_x(self):
        self.assertEqual(1, 1)  # NOQA: PT009

import unittest


class T(unittest.TestCase):
    def test_x(self):
        self.assertEqual(1, 1)  # noqa: E501
