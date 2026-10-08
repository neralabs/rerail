<?php
// Braced namespaces, the braced global namespace, and aliases local to each block.

namespace Shop\Cart {
    use Shop\Pricing\Calculator;
    use function Shop\Pricing\round_price;
    use const Shop\Pricing\CURRENCY;

    function add_item(Item $item): void
    {
        $calculator = new Calculator();
        round_price($calculator->total());
        add_item($item);
        remove_item($item);
        array_map('trim', []);
        echo CURRENCY, TAX_RATE, PHP_INT_SIZE, CART_LIMIT;
    }

    const CART_LIMIT = 10;
    define('Shop\Cart\TAX_RATE', 0.2);
}

namespace Shop\Checkout {
    $calculator = new Calculator();
    round_price(1.0);
    add_item(null);
    \Shop\Cart\add_item(null);
    echo CURRENCY, CART_LIMIT, TAX_RATE, M_PI;
}

namespace {
    use Shop\Cart\Item;

    $item = new Item();
    add_item($item);
    Shop\Cart\add_item($item);
    echo Shop\Cart\CART_LIMIT, CART_LIMIT, PHP_OS;
    $date = new DateTime();
}
