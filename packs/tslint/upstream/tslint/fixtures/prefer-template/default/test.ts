"a" + x;
x + "a";

"a" + x + "b";

x + "a" + y;

"a" + x + y;

"a" + x + "b" + y + "c";

`a` + x + `b${y}c`;

"a" + "b" + "c";

x`a` + y; // OK, can't simplify something with a tag

"a" + "b"; // OK to concatenate strings without newlines.

"a\n" + "b";

"a" + "\nb";

1 + 1;
1 + 1 + "a";
x + y;

