export var x: number;

declare global {
    var x: number;
}

declare namespace foo.global {
    var x: number;
    export namespace global {
        var x: number;
    }
}

