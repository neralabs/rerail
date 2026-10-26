import sqlite3
import unittest

from api.inventory import Inventory
from api.lookup import find_item


class InventoryTest(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.inventory = Inventory(self.conn)

    def tearDown(self):
        self.conn.close()

    def test_add_and_get(self):
        self.inventory.add_item("A1", "Bolt", 3)
        self.assertEqual(self.inventory.get_item("A1"), {"sku": "A1", "name": "Bolt", "quantity": 3})
        self.assertIsNone(self.inventory.get_item("missing"))

    def test_quantity_defaults_to_zero(self):
        self.inventory.add_item("A2", "Nut")
        self.assertEqual(self.inventory.get_item("A2")["quantity"], 0)

    def test_duplicate_sku_rejected(self):
        self.inventory.add_item("A1", "Bolt", 3)
        with self.assertRaises(ValueError):
            self.inventory.add_item("A1", "Other", 1)

    def test_negative_quantity_rejected(self):
        with self.assertRaises(ValueError):
            self.inventory.add_item("A1", "Bolt", -1)

    def test_adjust(self):
        self.inventory.add_item("A1", "Bolt", 3)
        self.assertEqual(self.inventory.adjust("A1", 4), 7)
        self.assertEqual(self.inventory.adjust("A1", -2), 5)
        self.assertEqual(self.inventory.get_item("A1")["quantity"], 5)

    def test_adjust_unknown_sku(self):
        with self.assertRaises(KeyError):
            self.inventory.adjust("missing", 1)

    def test_adjust_below_zero_keeps_quantity(self):
        self.inventory.add_item("A1", "Bolt", 3)
        with self.assertRaises(ValueError):
            self.inventory.adjust("A1", -4)
        self.assertEqual(self.inventory.get_item("A1")["quantity"], 3)

    def test_low_stock_sorted(self):
        self.inventory.add_item("C3", "Washer", 1)
        self.inventory.add_item("A1", "Bolt", 0)
        self.inventory.add_item("B2", "Nut", 9)
        self.assertEqual(self.inventory.low_stock(5), ["A1", "C3"])

    def test_remove(self):
        self.inventory.add_item("A1", "Bolt", 3)
        self.assertTrue(self.inventory.remove_item("A1"))
        self.assertFalse(self.inventory.remove_item("A1"))
        self.assertIsNone(self.inventory.get_item("A1"))

    def test_find_item(self):
        self.inventory.add_item("A1", "Bolt", 3)
        self.assertEqual(find_item(self.conn, "A1"), ("A1", "Bolt", 3))
        self.assertIsNone(find_item(self.conn, "missing"))

    def test_changes_are_committed(self):
        self.inventory.add_item("A1", "Bolt", 3)
        self.assertFalse(self.conn.in_transaction)


if __name__ == "__main__":
    unittest.main()
