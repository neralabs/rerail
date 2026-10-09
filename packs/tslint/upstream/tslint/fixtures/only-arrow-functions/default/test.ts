function foo(a: any) : any {
    return;
}

let a = () => {};
let b = function () {};

function c() {}

function () {
    // ...
}

((func) => func())(function e(): void {});

((func) => func())(() => {});

function* generator() {}
let generator = function*() {}

function hasThisParameter(this) {}
let hasThisParameter = function(this) {}

let usesThis = function() { this; }
let usesThis2 = function(foo = this) {}

let notUsesThis = function() {
    function f() { this; }
}
let notUsesThis2 = function() {
    return class { method() { this; } }
}

export function exported() {}

async function asyncFunction() {}

