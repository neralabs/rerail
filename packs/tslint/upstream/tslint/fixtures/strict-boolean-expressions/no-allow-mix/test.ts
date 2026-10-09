declare function get<T>(): T;

if (get<string | number>()) {}

if (get<string | boolean>()) {}

if (get<string | undefined>()) {}

if (get<boolean>()) {}
if (get<string>()) {}
if (get<RegExp | undefined>()) {}

if (get<"" | "foo">()) {}

// Mix of truthy values OK
if (get<"foo" | "bar" | undefined>()) {}

// Mix of falsy values not OK
if (get<"" | boolean>()) {}

// Mix of 1 falsy with any number of truthy OK
if (get<string | RegExp>()) {}

