class C {
    method(x: number) {}
    property: () => void;
    template(strs: TemplateStringsArray, x: any) {}
}

const c = new C();
[0].forEach(c.method);
[0].forEach(x => c.method(x));
[0].forEach(c.property);

c.template;
c.template`foo${0}`;
String.raw`${c.template}`;

interface I {
    foo(): void;
    bar: () => void;
}
declare var i: I;
i.foo;
i.bar;

c.method === i.foo;

// OK in condition
c.method ? 1 : 2;
1 ? c.method : c.method;
if (c.method) {}
while (c.method) {}
do {} while (c.method);
for (c.method; c.method; c.method) {}


[0].forEach(c.method || i.foo);
[0].forEach(c.method.bind(c));

<button onClick={c.method}>Click me!</button>;

class Validators {
    static required() {
        return null;
    }
    static compose(...args: Function[]) {}
}

Validators.compose(Validators.required);

const a = {
    getText() {
        return 'text';
    }
};

delete a.getText;

