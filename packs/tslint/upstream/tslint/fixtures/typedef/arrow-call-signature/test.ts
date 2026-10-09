function foge() {}

class TsLint {
    helloWorld() {
        return 'Hello World';
    };

    arrowHelloWorld = () => {
        return this.helloWorld();
    };
}

var obj = {
    foge: function() {},
    bar: () => {}
}

setTimeout(() => {}, 1000);
setTimeout(function() {}, 1000);

const foo = n => n+1

someFunc(n => n+1);
someFunc(n => {});
