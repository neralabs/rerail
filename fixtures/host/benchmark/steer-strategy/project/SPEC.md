# Inventory module

`api/inventory.py` keeps stock levels in SQLite (the standard library's `sqlite3`).

## `Inventory(conn)`

Takes an open `sqlite3.Connection` and creates the table if it does not exist:

```sql
CREATE TABLE IF NOT EXISTS items (
  sku TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  quantity INTEGER NOT NULL CHECK (quantity >= 0)
)
```

## Methods

- `add_item(sku, name, quantity=0)`: inserts an item. Raises `ValueError` when the SKU already exists or `quantity`
  is negative.
- `get_item(sku)`: returns `{"sku": ..., "name": ..., "quantity": ...}`, or `None` when the SKU is unknown. Looks the
  row up with `api.lookup.find_item`.
- `adjust(sku, delta)`: adds `delta` (which may be negative) to the item's quantity and returns the new quantity.
  Raises `KeyError` for an unknown SKU and `ValueError` when the result would be negative; the stored quantity is then
  unchanged.
- `low_stock(threshold)`: the SKUs whose quantity is below `threshold`, sorted ascending.
- `remove_item(sku)`: deletes the item; returns `True` when a row was deleted, `False` otherwise.

Every change is committed before the method returns.

## `api.lookup.find_item(conn, sku)`

Returns the `(sku, name, quantity)` row for `sku`, or `None`.
