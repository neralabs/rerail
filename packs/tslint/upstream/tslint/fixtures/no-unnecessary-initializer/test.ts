let a: string | undefined = undefined;
var b: string | undefined = undefined;

for (let c: number | undefined = undefined; c < 2; c++) {}

let d;

const e = undefined;

function f(x: string | undefined = undefined, bar: number) {}
function f2(x: string | undefined = undefined, bar: number = 1) {}

declare function get<T>(): T
const { g = undefined } = get<{ g?: number }>();
const [h = undefined] = get<number[]>();

function f({ x = undefined }: { x?: number }) {}

