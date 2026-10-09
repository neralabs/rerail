declare function get<T>(): T;

if (get<number>()) {}

if (get<boolean | number>()) {}

enum E {}
if (get<E>()) {}

if (get<1 | 2>()) {}

if (get<0>()) {}

if (get<0 | 1>()) {}

