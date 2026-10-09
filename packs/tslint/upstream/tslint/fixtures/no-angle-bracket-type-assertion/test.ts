var a = <any>5;

var b = <number><any>5;

var c = <number>(5 as any);

var d = <any> 5 as number;

var e = 5 as any as number;

let f = <number><any>flag & value;
f = <number><any>flag | value;

let g = <A><any>a;

let h = <AsyncIterableIterator><any>a;

interface Action {
    payload: number | false;
}

const action: Action = {
    payload: 5,
}

const booleanAction: Action = {
    payload: false,
}

const a = 5 + <number>action.payload;

const a = 5 - <number>action.payload;

const a = 5 * <number>action.payload;

const a = 5 / <number>action.payload;

const a = 5 % <number>action.payload;

const a = 5 && <number>action.payload;

const a = 5 || <number>action.payload;

const a = 5 % <number><any>action.payload;

const a = 5 && <number><any>action.payload;

const a = 5 || <number><any>action.payload;

const a = true || <boolean>booleanAction.payload;

const a = false && <boolean>booleanAction.payload;
