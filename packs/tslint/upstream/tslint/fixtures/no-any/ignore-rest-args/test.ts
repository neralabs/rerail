var x: any; // error

function foo(a: any) : any { // 2 errors
    return;
}

const fooArrow(a: any[]) => {
    return;
}

function bar(...a: any[]) {
    return;
}

const barArrow = (...a: any[]): any => {
    return;
}

const function(...a: { [key: any]: any }[]) {
    return;
}

let a: any = 2, // error
    b: any[] = 4; // error

let {a: c, b: d}: {c: any, d: number} = {c: 99, d: 100};  // error

