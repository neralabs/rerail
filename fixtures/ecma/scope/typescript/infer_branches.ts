type T1<X> = X extends infer Function ? Function : Function;
type T2<X> = X extends [infer Head, ...infer Tail] ? [Head, Tail] : [Head, Tail];
type T3<X> = X extends infer U extends string ? U : U;
type T4<X> = X extends Array<infer E> ? (E extends infer F ? [E, F] : [E, F]) : [E, F];
type T5<X> = X extends infer U ? X : U;
type T6<X> = infer_left<X> extends infer V ? V : V;
type T7<X> = X extends (k: infer K) => void ? K : K;
type T8<X> = X extends { [key: string]: (a: infer A) => infer R } ? [A, R] : A;
