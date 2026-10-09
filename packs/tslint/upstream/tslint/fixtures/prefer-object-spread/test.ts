const form = Object.assign({}, this);
const foo = Object.assign(bar, this);
const foo = Object.assign({}, bar, baz, this);
const original = {a: 1, b:2};
Object.assign({}, original, {c: 3});
Object.assign({a: 1, b: 2}, {c: 3});
Object.assign({a:1}, ...[{b: 2}, {c: 3}])
Object.assign(original, {c: 3});
var result = Object.assign(original, {c: 3});
result = Object.assign(original, {c: 3});
var result = Object.assign({}, original, {c: 3});
foo(Object.assign(original, {c: 3}));
[Object.assign(original, {c: 3})];
({
    foo: Object.assign(original, {c: 3})
})

Object.assign({}, {}, {a: 1,},);
Object.assign({a: 1}, foo ? {b: 2} : {c: 3});
Object.assign({a: 1, ...{b: 2}});
Object.assign({}, {});
Object.assign({}, {},);

// allowed
result = Object.assign(new Foo(), {});
result = Object.assign(createFoo(), {});
result = Object.assign(function() {}, {});
result = Object.assign(() => {}, {});

let fn = () => Object.assign({}, original);
