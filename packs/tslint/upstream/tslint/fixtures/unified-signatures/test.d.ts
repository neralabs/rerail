interface I {
    // For 3 or more overloads, mentions the line.
    a0(): void;
    a0(x: string): string;
    a0(x: number): void;

    // Error for extra parameter.
    a1(): void;
    a1(x: number): void;

    // No error for arity difference greater than 1.
    a2(): void;
    a2(x: number, y: number): void;

    // Error for arity difference greater than 1 if the additional parameters are all optional/rest.
    a3(): void;
    a3(x: number, y?: number, ...z: number[]): void;

    // No error for different return types.
    a4(): void;
    a4(x: number): number;

    // No error if one takes a type parameter and the other doesn't.
    a5<T>(x: T): T;
    a5(x: number): number;

    // Error if only one defines a rest parameter.
    b(): void;
    b(...x: number[]): void;

    // No error if one is a rest parameter and other isn't.
    b2(x: string): void;
    b2(...x: number[]): void;

    // No error if both are rest parameters. (https://github.com/Microsoft/TypeScript/issues/5077)
    b3(...x: number[]): void;
    b3(...x: string[]): void;

    // Error if only one defines an optional parameter.
    c(): void;
    c(x?: number): void;

    // Error if both are optional.
    c2(x?: number): void;
    c2(x?: string): void;

    // No error if one is optional and the other isn't.
    c3(x: number): void;
    c3(x?: string): void;

    // Error for different types (could be a union)
    d(x: string): void;
    d(x: number): void;

    // No error if they differ by 2 or more parameters.
    d2(x: string, y: number): void;
    d2(x: number, y: string): void;
}

// Works for type literal too.
type T = {
    // Works for call signature too.
    (): void;
    (x: number): void;
}

declare class C {
    // Works for constructor.
    constructor();
    constructor(x: number);
}

declare class D {
    // No conflict between static/non-static members.
    static a();
    a(x: number);
}

// Allow separate overloads if one is generic and the other isn't.
interface Generic<T> {
    x(): void;
    x(x: T[]): void;

    y(x: T[]): void;
    y(x: T): void;
}

