if (2) {}
if (2 > 1) {}
if (2 < 1) {}
if (2 >= 1) {}
if (2 <= 1) {}
if (2 == 1) {}
if (2 === 1) {}
if (2 != 1) {}
if (2 !== 1) {}
if (2 > undefined) {}
if (undefined === 1) {}
if (2 > undefined) {}
if (true) {}
if (true > false) {}
if (true < false) {}
if (true >= false) {}
if (true <= false) {}
if (true == false) {}
if (true === false) {}
if (true != false) {}
if (true !== false) {}
if ('') {}
if ('' > '') {}
if ('' < '') {}
if ('' >= '') {}
if ('' <= '') {}
if ('' == '') {}
if ('' === '') {}
if ('' != '') {}
if ('' !== '') {}
if ({}) {}
if ({} > {}) {}
if ({} < {}) {}
if ({} >= {}) {}
if ({} <= {}) {}
if ({} == {}) {}
if ({} === {}) {}
if ({} != {}) {}
if ({} !== {}) {}
if ([] === []) {}

if (3 > 2 || 2 > 1 && true === true) {}
if ('' > '' || 2 > 1 || {} > {}) {}
if ('' > '' && 2 > 1 && {} > {}) {}

if ({} === null) {}
if (null === {}) {}
if ({} === undefined) {}
if (undefined === {}) {}

function sameObject<T>(a: T, b: T): boolean {
    return a === b;
}

function sameObject<T>(a: any, b: any): boolean {
    return a === b;
}

type myNumber = number;
const a1: myNumber = 1
const a2: myNumber = 2

if (a1 < a2) {}
if (a2 < a1) {}

type myString = string;
const b1: myString = ''
const b2: myString = ''

if (b1 === b2) {}
if (b2 === b1) {}

type myObject = Object;
const c1: myObject = {}
const c2: myObject = {}

if (c1 === c2) {}
if (c2 === c1) {}

const d1: any = 'string'
const d2: any = 2
if (d1 === d2) {}
if (d2 === d1) {}

enum TestNumericEnum {
    One = 1,
    Two = 2,
}

const e1: TestNumericEnum = TestNumericEnum.One

if (e1 === TestNumericEnum.Two) {}
if (TestNumericEnum.Two === e1) {}
if (e1 > TestNumericEnum.Two) {}
if (TestNumericEnum.Two > e1) {}

const f1: TestNumericEnum | undefined
const f2: TestNumericEnum | undefined

if (f1 === f2) {}
if (f2 === f1) {}

enum TestStringEnum {
    One = 'one',
    Two = 'two',
}

const g1: TestStringEnum = TestStringEnum.One

if (g1 === TestStringEnum.Two) {}
if (TestStringEnum.Two === g1) {}
if (g1 > TestStringEnum.Two) {}
if (TestStringEnum.Two > g1) {}

const h1: string | number = Math.random() > 0.5 ? 'text' : 5;
const h2: string | number = Math.random() > 0.5 ? 'test' : 2;
if (h1 > h2) {}
if (h2 > h1) {}
if (h1 === h2) {}
if (h2 === h1) {}

if (undefined === null) {}
if (null !== undefined) {}

if (1 > null) {}
if (null > 1) {}
if (1 >= undefined) {}
if (undefined >= 1) {}
if (undefined <= null) {}
if (null >= undefined) {}

const func = (param?: boolean): boolean => {
  return param === undefined ? false : param;
}
const func = (param?: boolean): boolean => {
  return param === null ? false : true;
}
const func = (param: number): number => {
  return param + 1;
}
const func = (arg1: number, arg2: number): boolean => {
  return arg1 === arg2;
}
const func = (param: string | null = null) => {
    if (param !== null) {}
    if (param !== undefined) {}
}

