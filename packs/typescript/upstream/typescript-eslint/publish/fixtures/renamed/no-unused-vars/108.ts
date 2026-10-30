
export type Qnpd<U> = U extends (l: infer K) => void ? K : never;
    