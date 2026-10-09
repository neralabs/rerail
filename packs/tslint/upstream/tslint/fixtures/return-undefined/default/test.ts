class Promise<T> {
    then(): Promise<T>;
}
function valueWrong(): number | undefined {
    return;
}
function valueRight(): number | undefined {
    return undefined;
}

function voidWrong(): void {
    return undefined;
}
function voidRight(): void {
    return;
}

// Infers type from context.
[].forEach(() => {
    return undefined;
});
[].map<number | undefined>(() => {
    return;
});

type Cb = () => void;
declare function badContextualType(cb: Cb | Cb[]): void;
// Uses typeAtLocation instead of contextual type.
badContextualType(() => {
    if (1 === 2) return 1;
    else return;
});

// Allow anything in callback taking 'any'.
function takesAnyCb(cb: () => any): void;
takesAnyCb(() => { return; });
takesAnyCb(() => { return undefined; });
takesAnyCb(() => {
    if (1 === 2) return;
    else return 1;
});
takesAnyCb(() => {
    if (1 === 2) return;
    else return undefined;
});
takesAnyCb((): void => {
    if (1 === 2) return;
    else return undefined;
});

async function promiseVoid(): Promise<void> {
    if (1 === 2) return;
    else return undefined;
}

async function promiseValue(): Promise<number | undefined> {
    if (1 === 2) return 1;
    else return;
}

declare function test(cb: () => Promise<void> | void): void;

test(async () => {
    if (1 === 2) return;
    else return undefined;
});


function * generator(returnNothing: boolean) {
    if (returnNothing) return;
    yield 1;
    return yield * [2, 3];
}

class ClassWithGeneratorMethod {
    * generatorMethod(returnNothing: boolean) {
        if (returnNothing) return;
        yield 1;
    }
}

