function foo(evt: Event) {
    let length: number = 1;
    console.log(length);

    Event.target;

    event.target;
}

console.log(length);

import { name } from "./foo";
import "./custom-global"
console.log(name);

console.log(badGlobal);

console.log(goodGlobal);

let { foo = event } = bar

let { foo = (() => event)() } = bar;

let foo: typeof event;

function rest() {
    const { bar, ...event } = foo;
}

function nested() {
    const { bar, event: { x: y } } = foo;
}

function initializer() {
    const { bar, length = () => event } = foo;
}

function nestedInitializer() {
    const { bar, length: { x: y = () => event } } = foo;
}

function computedProperty() {
    const { [event.type]: x } = foo;
}

function typeTest() {
    type name = any;
}

function functionTest () {
    const foo = name;
    
    function name () {

    }
}

function interfaceTest() {
    var foo: name;
    interface name {}
}

function classTest() {
    const foo = new name();
    class name {}
}