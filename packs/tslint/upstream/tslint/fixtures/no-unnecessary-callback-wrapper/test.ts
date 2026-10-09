x => f(x);

(x, y) => f(x, y);

(x, y) => f(x, y, 0);
(x, y) => f(y, x);

(x, y) => f<T>(x, y);

(x, ...y) => f(x, y);

(...args) => f(...args);

// Ignore if function is not an identifier
(x) => obj.f(x);
(x) => obj["f"](x);
x => foo()(x);
// x is not only used as argument
(x) => obj[x](x);
x => x(x);

// allow async arrows
async (x) => f(x);

// Not bothering to catch this case.
({ x, y }) => f({ x, y });

