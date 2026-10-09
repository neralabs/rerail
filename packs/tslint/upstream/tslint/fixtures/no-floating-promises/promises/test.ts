class Promise<T> {
    then(): Promise<T>;
}

function returnsPromiseFunction() {
    return new Promise<void>();
}

const returnsPromiseVariable = () => new Promise<void>();

class ReturnsPromiseClass {
    returnsPromiseMemberFunction() {
        return new Promise<void>();
    }

    returnsPromiseMemberVariable = () => new Promise();

    static returnsPromiseStaticFunction = () => new Promise();
}

let a = returnsPromiseFunction();
let b = returnsPromiseVariable();
let c = new ReturnsPromiseClass().returnsPromiseMemberFunction();
let d = new ReturnsPromiseClass().returnsPromiseMemberVariable();
let e = ReturnsPromiseClass.returnsPromiseStaticFunction();

returnsPromiseFunction();

returnsPromiseFunction().then();

returnsPromiseVariable();

returnsPromiseVariable().then();

new ReturnsPromiseClass().returnsPromiseMemberFunction();

new ReturnsPromiseClass().returnsPromiseMemberFunction().then();

new ReturnsPromiseClass().returnsPromiseMemberVariable();

new ReturnsPromiseClass().returnsPromiseMemberVariable().then();

ReturnsPromiseClass.returnsPromiseStaticFunction();

ReturnsPromiseClass.returnsPromiseStaticFunction().then();

(function () {
    let a = returnsPromiseFunction();

    returnsPromiseFunction();

    returnsPromiseFunction().then();

    new ReturnsPromiseClass().returnsPromiseMemberFunction();

    new ReturnsPromiseClass().returnsPromiseMemberFunction().then();

    new ReturnsPromiseClass().returnsPromiseMemberVariable();

    new ReturnsPromiseClass().returnsPromiseMemberVariable().then();

    ReturnsPromiseClass.returnsPromiseStaticFunction();

    ReturnsPromiseClass.returnsPromiseStaticFunction().then();
})();

(() => {
    let a = returnsPromiseFunction();

    returnsPromiseFunction();

    returnsPromiseFunction().then();

    new ReturnsPromiseClass().returnsPromiseMemberFunction();

    new ReturnsPromiseClass().returnsPromiseMemberFunction().then();

    new ReturnsPromiseClass().returnsPromiseMemberVariable();

    new ReturnsPromiseClass().returnsPromiseMemberVariable().then();

    ReturnsPromiseClass.returnsPromiseStaticFunction();

    ReturnsPromiseClass.returnsPromiseStaticFunction().then();
})();

[].push(returnsPromiseFunction());

[].push(returnsPromiseFunction().then());

[].push(ReturnsPromiseClass.returnsPromiseStaticFunction());

[].push(ReturnsPromiseClass.returnsPromiseStaticFunction().then());

while (returnsPromiseFunction());

while (returnsPromiseFunction().then());

for (let i = 0; i < returnsPromiseFunction(); i += 1);

for (let i = 0; i < returnsPromiseFunction().then(); i += 1);

for (let i in returnsPromiseFunction());

for (let i in returnsPromiseFunction().then());

for (const promise of returnsPromiseFunction());

for (const promise of returnsPromiseFunction().then());

let promise = Math.random() > .5
    ? returnsPromiseFunction()
    : returnsPromiseFunction().then();

declare var foo: boolean;
if (foo)
    returnsPromiseFunction();
switch(foo) {
    case true:
        returnsPromiseFunction();
        break;
    case false: {
        returnsPromiseFunction();
        break;
    }
}

returnsPromiseFunction().catch(e => { console.error(e.stack); });

returnsPromiseFunction().then(() => {}, e => { console.error(e.stack); });

