let x = (y = 1, z = 2);

// Error prone: forgot to add parens around arguments.
(x, y => x + y)(a, b);

foo((bar, baz));

switch (blah) {
    case 1, 2: // equals `case 2` - probably intended `case 1: case2:`
        return true;
    case 3:
        return false;
}

for(let i = 0, j = 0; i < 10, j < 10; i++, j++) {
}

