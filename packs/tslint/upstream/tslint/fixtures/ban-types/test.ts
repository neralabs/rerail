let a: Object;
let b: {c: String};
function foo(a: String) {}
'a' as String;
let c: F;
let d: Foooooo;

// no warning for separately scoped types
let e: foo.String;

// no warning for actual uses.
let f = Object();
let g = Object.create(null);
let h = String(false);