import unittest


class A(unittest.TestCase):
    def test(self):
        self.assertEquals(1, 1)

import unittest


class A(unittest.TestCase):
    def test(self):
        self.assertEquals(1, 1)  # noqa

import unittest


class A(unittest.TestCase):
    def test(self):
        self.assertEquals(1, 1)  # noqa: UP005

import unittest


class A(unittest.TestCase):
    def test(self):
        self.assertEquals(1, 1)  # NOQA: UP005

import unittest


class A(unittest.TestCase):
    def test(self):
        self.assertEquals(1, 1)  # noqa: E501
