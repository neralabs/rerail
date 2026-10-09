var foo = function(): void {};
let foo = function(): void {};
const foo = function(): void {};

var foo = (): void => {};
let foo = (): void => {};
const foo = (): void => {};

class Foo {
    foo = (): void => {};
    foo = function(): void {};
}

const foo: () => void = (): void => {};
const foo: () => void = function(): void {};

var noTypeDef = 'Should still fail';

class NoTypeDef {
    public noTypeDef = 'Should still fail';
}
