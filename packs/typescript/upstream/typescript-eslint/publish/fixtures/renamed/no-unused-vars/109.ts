
export type Ormq<T> = T extends { [g: string]: infer D } ? D : never;
    