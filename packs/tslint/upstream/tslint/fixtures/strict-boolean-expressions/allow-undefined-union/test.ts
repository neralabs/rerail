declare function get<T>(): T;

if (get<RegExp | undefined>()) {}

declare const bu: boolean | undefined;
if (get<boolean | undefined>()) {}

// If it's always undefined, testing for it is wrong.
if (get<undefined>()) {}

if (get<void>()) {}

if (get<RegExp | null>()) {}

// Type of the condition is actually boolean | RegExp, but OK since we check each part separately.
if (get<RegExp | undefined>() || get<boolean>()) {}

// This still fails of course!
if (get<RegExp>() || get<RegExp>()) {}

if (get<number | undefined>()) {}
