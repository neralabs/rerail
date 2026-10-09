var foo;

function tmp(t: any) {
    var x = 3;
}

var i,
    j;

var [a, b] = [1, 2];

for (var n; false;);
for (var n1 in foo);
for (var n2 of foo);

export var exportVar0 = 0;
export   var  exportVar1 = 1;
export   var  exportVar2 = 2,
         exportVar21 = 21;
export /* var tst */ var   exportVar3 = 3;

module quz {
    export var i;
}

declare var tmp2: any;

let bar;
const qux;

let k,
    l;

let [x, y] = [1, 2];

for (n; false;);
for (let n; false;);
for (let name in foo);
for (let name of foo);
for (const name in foo);
for (const name of foo);

export let exportLet = 1;
export const exportConst = 1;
