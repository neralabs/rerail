console.log(-1, 0, 1);
console.log(42.42);
console.log(-0);
const a = 1337;
const b = {
    a: 1338,
    b: 0,
}
b.b = 1339;

console.log(1340);
for(let i = 0;i>1341;++i) {
}

throw 1342;

class A {
    static test = 1337;

    constructor (private a = 1337) {
    }
}

enum Test {
    key = 1337,
}

const a = 0.1234567801234567890;
a.toFixed(2);
a.toFixed(5);
a.toFixed(10);

a.toString(10);
a.toString(36);

a.toPrecision(3)
a.toPrecision(10)
a.toPrecision(20)

const b = 77.1234;
b.toExponential(4);
b.toExponential(2);
