async function correct(foo: AsyncIterableIterator<string>) {
    for await (const element of foo) {}
}

async function correct2() {
    for await (const element of asyncGenerator()) {}
}

async function correct(foo: AsyncIterable<string>) {
    for await (const element of foo) {}
}

async function correct3(foo: AsyncIterableIterator<string> | AsyncIterableIterator<number>) {
    for await (const element of foo) {}
}

async function incorrect(foo: Array<Promise<string>) {
    for await (const element of foo) {}
}

async function incorrect2(foo: IterableIterator<Promise<string>>) {
    for await (const element of foo) {}
}

async function incorrect3() {
    for await (const element of asyncGenerator) {}
}

async function incorrect4() {
    for await (const element of generator()) {}
}

async function incorrect5(foo: Iterable<string>) {
    for await (const element of foo) {}
}

async function* asyncGenerator() {
    yield 1;
}

function* generator() {
    yield 1;
}

