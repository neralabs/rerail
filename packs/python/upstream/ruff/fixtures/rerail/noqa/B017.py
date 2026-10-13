import unittest
class T(unittest.TestCase):
    def test(self):
        with self.assertRaises(Exception):
            pass

import unittest
class T(unittest.TestCase):
    def test(self):
        with self.assertRaises(Exception):  # noqa
            pass

import unittest
class T(unittest.TestCase):
    def test(self):
        with self.assertRaises(Exception):  # noqa: B017
            pass

import unittest
class T(unittest.TestCase):
    def test(self):
        with self.assertRaises(Exception):  # NOQA: B017
            pass

import unittest
class T(unittest.TestCase):
    def test(self):
        with self.assertRaises(Exception):  # noqa: E501
            pass
