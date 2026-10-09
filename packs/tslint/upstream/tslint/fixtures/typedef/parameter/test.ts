var NoTypesFn = function (a, b) {
}

var NoTypesArrowFn = (a, b) => {}

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
