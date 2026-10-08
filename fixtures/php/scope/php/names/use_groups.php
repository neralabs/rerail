<?php
// Group `use` statements with a statement-wide type and nested prefixes, multiple aliased
// clauses, and the names they import read back. (tree-sitter-php 0.24.2 reads neither a group
// prefix with a leading backslash nor a group's trailing comma.)

namespace Shop;

use function Util\Strings\{slugify, truncate as cut};
use const Config\{DEBUG, Limits\MAX_ITEMS};
use Domain\{Order, Order\Line as OrderLine};
use Domain\Customer as Buyer, \Domain\Address;

slugify('a');
cut('b', 1);
echo DEBUG, MAX_ITEMS;
new Order();
new OrderLine();
new Buyer();
new Address();
