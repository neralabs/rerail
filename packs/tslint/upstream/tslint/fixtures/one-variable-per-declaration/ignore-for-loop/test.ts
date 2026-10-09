// valid cases
for (var i = 0; i > 1; i++) {}
for (let i = 0; i > 1; i++) {}
for (const i = 0; i > 1;) {}
for (var i = 0, j = 0; i > 1; i++) {}
for (let i = 0, j = 0; i > 1; i++) {}
for (const i = 0, j = 0; i > 1;) {}

// invalid cases
var foo1, foo2;
var foo3: number, foo4: number;
var foo5 = 1, foo6 = 1;

let foo7, foo8;
let foo9: number, foo10: number;
let foo11 = 1, foo12 = 1;

const foo13: number = 1, foo14: number = 1;
const foo15 = 1, foo16 = 1;


