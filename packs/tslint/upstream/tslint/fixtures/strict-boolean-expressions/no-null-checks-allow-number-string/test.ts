declare function get<T>(): T;

if (get<number>()) {}

if (get<string>()) {}

enum E {}
if (get<E>()) {}
