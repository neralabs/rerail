interface I {
    b;
    a;
}

declare function f(i: I): void;

const a = 0, b = 0;

f({ b, a });

f({ a, b });

// Resets ordering after spread operator.

f({ a, ...x, b });

f({ a, ...x, a, b });


// Methods and getters/setters work like any other key.

f({ b() {}, a() {} });

f({ a() {}, b() {} });

f({
    get b() {},
    a,
    set b(v) {},
});

f({
    get b() {},
    set b() {},
    a,
});

// Ignores computed properties. Does not ignore string / number keys.

interface J {
    "foo";
    2;
    [Symol.iterator];
}
declare function j(j: J): void;
j({ [Symbol.iterator]: 1, "foo": 1, 2: 1 });
j({ [Symbol.iterator]: 1, 2: 1, "foo": 1 });

// Works with anonymous type too.
type T = { b, a };
const o: T = { a, b };

const o: { b, a } = { a, b };

// Non-alphabetical ordering is fine, even if it can't find a type.

const o = {
    b,
    a,
};

