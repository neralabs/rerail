
type T<Symbol> = Symbol;
type U<UU> = UU extends T<infer Function> ? Function : never;
    