function f1(foo = "") {}

function f2([foo = 0]: number[]) {}

function f3(foo: number) {
    foo;
}

let noParam: number = 0;
noParam++;
noParam = 0;
let [alsoNoParam] = [0];
--alsoNoParam;

function f4(foo: number, {bar}: {bar: number}, baz: string) {
    --noParam;
    alsoNoParam += alsoNoParam;
    foo += 1;
    foo++;
    --foo;
    foo += foo;
    ({foo} = {foo: 1});
    for (bar of arr) {}
    for (baz in obj) {}
}

