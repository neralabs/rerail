declare function get<T>(): T;

if (get<RegExp | null>()) {}

if (get<boolean | null>()) {}

if (get<null>()) {}

if (get<RegExp | undefined>()) {}
