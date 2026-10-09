declare function get<T>(): T;

if (get<string>()) {}

if (get<boolean | string>()) {}

if (get<"foo" | "bar">()) {}

if (get<"">()) {}

if (get<"foo" | "">()) {}
