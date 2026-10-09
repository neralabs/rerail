[1, , 3];

// Destructuring allowed.
const [foo, , bar] = [];
const [{foo: [, bar]}] = [];
const [foo = [, 1]] = [];

[foo, , (bar)] = [];
[{foo: [, bar]}] = [];
[foo = [, 1]] = [];
[foo = bar([,])] = [];

[1,,];

[1,,,];

