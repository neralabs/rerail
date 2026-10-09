// valid cases
var foo1;
var foo2: number;
var foo3 = 1;

let foo4;
let foo5: number;
let foo6 = 1;

const foo7: number = 1;
const foo8 = 1;

for (var i = 0; i > 1; i++) {}
for (let i = 0; i > 1; i++) {}
for (const i = 0; i > 1;) {}

// invalid cases
var foo9, foo10;
var foo11: number, foo12: number;
var foo13 = 1, foo14 = 1;

let foo15, foo16;
let foo17: number, foo18: number;
let foo19 = 1, foo20 = 1;

const foo21: number = 1, foo22: number = 1;
const foo23 = 1, foo24 = 1;

for (var i = 0, j = 0; i > 1; i++) {}
for (let i = 0, j = 0; i > 1; i++) {}
for (const i = 0, j = 0; i > 1;) {}


