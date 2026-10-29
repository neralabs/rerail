import unittest


class T(unittest.TestCase):
    def test_x(self):
        with self.assertRaises(ValueError):  
            raise ValueError

import unittest


class T(unittest.TestCase):
    def test_x(self):
        with self.assertRaises(ValueError):    # noqa
            raise ValueError

import unittest


class T(unittest.TestCase):
    def test_x(self):
        with self.assertRaises(ValueError):    # noqa: PT027
            raise ValueError

import unittest


class T(unittest.TestCase):
    def test_x(self):
        with self.assertRaises(ValueError):    # NOQA: PT027
            raise ValueError

import unittest


class T(unittest.TestCase):
    def test_x(self):
        with self.assertRaises(ValueError):    # noqa: E501
            raise ValueError
