interface I {
    (): void;
}

interface J extends Function {
    (): void;
}

interface K<T> {
    <U extends T>(param1: U, param2: T): U;
}

interface L<T> {
    (): T
}

type T = {
    (): void;
};

type U = {
    <T>(t: T): T;
}

var fn: {(): void;};
function f(x: { (): void }): void;
function f(x: string | { (): void }): void;
function f(x: string | ({ (): void })): void;
function f(x: { (): string } | { (): number }): void;
function f(x: string & { (): void }): void;
function f(x: { (): string } & { (): number }): void;
function f(x: { (): string }[]): void;

// Overloads OK
interface K {
    (x: number): number;
    (x: string): string;
}

// handle modifiers
export interface I0 {
    (): void;
}

export interface I1 {
    new (): void;
}

export type T = {
    (): void;
}

