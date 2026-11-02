
type T<Symbol> = Symbol;
type Z<ZX> = ZX extends T<infer Function> ? Function : never;
    