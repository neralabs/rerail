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

someFunc(n => n+1);
someFunc(n => {});

class AComponent {
    private _test(): void {
        this._foo.test({
            callback: () => {
                // asd
            },
        });
    }
}

function foo(bar: any) :void {}
foo({bar: ()=>{}});
