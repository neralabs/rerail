declare function get<T>(): T;

declare const bu: boolean | undefined;
if (get<boolean | undefined>()) {}
if (get<true | undefined>()) {}
if (get<true | false | undefined>()) {}
if (get<true | false | undefined | void>()) {}

if (get<true>()) {}
if (get<true | true>()) {}
if (get<false | undefined>()) {}
if (get<undefined>()) {}

if (get<void>()) {}

if (get<RegExp | undefined>()) {}
if (get<RegExp | null>()) {}

// Type of the condition is actually boolean | RegExp, but OK since we check each part separately.
if (get<RegExp | undefined>() || get<boolean>()) {}

// This still fails of course!
if (get<RegExp>() || get<RegExp>()) {}

if (get<number | undefined>()) {}
