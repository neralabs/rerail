interface I {
    new(): I;
    // OK if return type is not the interface.
    new(): {};
    constructor(): void;
}

// Works for generic type.
interface G {
    new<T>(): G<T>;
}

type T = {
    // `new` OK in type literal (we don't know the type name)
    new(): T;
    // `constructor` still flagged.
    constructor(): void;
}

class C {
    new(): C;
    // OK if there's a body
    new() {}
}

