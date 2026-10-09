declare function get<T>(): T;

if (get<{ x: number }>) {}

if (get<string>()) {}

if (get<number>()) {}

enum E {}
if (get<E>()) {}

if (get<E | { x: number }>()) {}
