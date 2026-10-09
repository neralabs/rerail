"use strict";
'use asm';
"ngInject";
'';

function fun1() {
    "use strict";
    'someOtherDirective';
    return 0;
}

(function() { "directive";
'foo'
'directive2'
console.log('foo');
'notdirective';
})();

const a = () => {
'use strict'; "use cool"; "use lint"; var a = 1; "notdirective"; }

function fun2(a: number) {
    return 0;
}

function fun3(a: number, b: number) {
    return 0;
}

namespace Fam { 'use strict'; 'use cool'; }
module Bam { 'use strict'; 'use cool'; }
namespace Az.Bz.Cz.Dz {
    'ngInject';
}

class Foo {
    constructor() {
        "ngInject";
        var a = 1;
        'notdirective';
    }

    bar() {
        'use strict';
    }

    get baz() {
        'use asm';
    }

    set baz(newValue) {
        "use asm";
    }
}

// valid code:

var i: number;
var j = 3;
i = 1 + 2;
j = fun1();
fun1();
fun2(2);
fun3(2, fun1());
i++;
i += 2;
--i;
i <<= 2;
i = fun1() + fun1();
j = (j === 0 ? 5 : 6);
(j === 0 ? fun1() : fun2(j));
(a => 5)(4);
var obj = {};
delete obj.key;
function* g(): Iterable<number> {
    for (let i = 0; i < 100; i++) {
        yield i;
    }
}

async function f(foo: Promise<void>): Promise<number> {
    await foo;
    return 0;
}

new Foo();

// invalid code:

5;
i;
3 + 5;
fun1() + fun1();
fun2(i) + fun3(4,7);
fun1() + 4;
4 + fun2(j);
(j === 0 ? fun1() : 5);
(j === 0 ? i : fun2(j));
a => fun2(a);
() => {return fun1();};
"use strct";

afterEach((el) => {
   el && el.remove();
});

checkParams((a, b) => {
   (a || required('a')) && (b || required('b'));
});

checkParams((a, b) => {
   ((a && b) || required('a, b'));
});

function interactionHandler(e) {
    // fails in all cases since logical NOT operator is redundant
    e && !e.preventDefault();
}

a && b ? a : b;

a && b;
a() && b;
a() && b();

let var1, var2;
if (foo)
    var1 = 1, var2 = 1;
else
    var1 = 2, var2;
var1, var2;

(0,eval)('"foobar";');
(foo,eval)('"foobar";');
(foo(),eval)('"foobar";');
(foo(), 0, eval)('"foobar";');
(0, 1, eval)('"foobar";');
(1, eval)('"foobar";');
(0, foo)('"foobar";');
var2 = (0, 1, 2);
(0, 1, 2);
(0, 1, foo());

var1 = void 0;
var1 = void(0);
var1 = void 1;
var1 = void(1);
var1 = void foo;
var1 = void foo === bar;

var1 = void (foo === bar);
var1 = void (foo = bar);
var1 = void foo.bar;
var1 = void foo();

void foo();
void (foo && foo());
void 0;
void (foo = bar);
void 1;
foo ? bar = foo : void 0;
var1 = foo ? void (foo && foo()) : void 0;

switch(l) {
  case 0, 1:
    return 'zero or one';
  default:
    return 'more than one';
}

let x = Math.pow((3, 5));
let a = [(3 + 4), ((1 + 1, 8) * 4)];

`foo`;
foo`bar`;

