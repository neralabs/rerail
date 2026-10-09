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

