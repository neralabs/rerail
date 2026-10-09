function func(): void {}

type NormalType = () => void;

let normalArrow = (): void => { }

let ughThisThing = void 0;

function takeThing(thing: undefined) { }
takeThing(void 0);

function takeVoid(thing: void) { }


const arrowGeneric = <T extends void>(arg: T) => { }
const arrowGeneric1 = <T = void>(arg: T) => { }
const arrowGeneric2 = <T extends void = void>(arg: T) => { }

function functionGeneric<T extends void>(arg: T) {}

function functionGeneric1<T = void>(arg: T) {}
function functionGeneric2<T extends void = void>(arg: T) {}

declare function functionDeclaration<T extends void>(arg: T): void;
declare function functionDeclaration1<T = void>(arg: T): void;
declare function functionDeclaration2<T extends void = void>(arg: T): void;


functionGeneric<void>(undefined);

declare function voidArray(args: void[]): void[];

let value = undefined as void;

let value = <void>undefined;

function takesThings(...things: void[]): void { }

type KeyofVoid = keyof void;

interface Interface {
    lambda: () => void;
    voidProp: void;
}

class ClassName {
    private readonly propName: void;
}

let voidPromise: Promise<void> = new Promise<void>(() => {});

let voidMap: Map<string, void> = new Map<string, void>();

let letVoid: void;

type VoidType = void;

class OtherClassName {
    private propName: VoidType;
}

type UnionType = string | number;
type UnionType2 = string | number | void;

type UnionType3 = string | (number & any | (string | void));

type IntersectionType = string & number & void;

function returnsVoidPromiseDirectly(): Promise<void> {
    return Promise.resolve();
}

async function returnsVoidPromiseAsync(): Promise<void> {}

type MappedType<T> = {
    [K in keyof T]: void;
}
type ConditionalType<T> = {
    [K in keyof T]: T[K] extends string ? void : string;
}

type ManyVoid = readonly void[];

function foo(arr: readonly void[]) { }

