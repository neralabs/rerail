subscribe(({ meta: newMeta }:  {meta: SessionMeta}) => {
    this.setCount(newMeta.match_count);
});

subscribe(({ meta: newMeta }) => this.setCount(meta.match_count));

$.x = 3;
import $ = require("./$");
var vara = varb, varb;

class Test {
    constructor() {
        this.a = 3;
    }

    private a: number;
}

var i = j;

class ClassA {
    prop: number;
    constructor(object: ClassB) {
        this.prop = object.prop;
    }
}

class ClassB {
    prop: number;
}

var j: number;

if (something) {
    var defined = 1;
} else {
    var defined;
}

testUndeclaredImports(); // Function early reference OK

function testUndeclaredImports() {
    console.log(foo1);
    console.log(foo2);
    console.log(foo3);
    map([], (x) => x);
}

import { default as foo1 } from "./lib";
import foo2 from "./lib";
import _, { map, foldl } from "./underscore";
import * as foo3 from "./lib";
import "./lib";

export default function () {
    //
};

export {
    undeclaredA,
    undeclaredB,
    undeclaredC,
    testUndeclaredImports
};

var undeclaredA = 42;
var { undeclaredB } = { undeclaredB: 43 };
var { undeclaredB: undeclaredA } = { undeclaredB: 43 };
var { undeclaredB: undeclaredA, undeclaredC } = { undeclaredB: 43, undeclaredC: 'hello' };
var { undeclaredB: undeclaredA, undeclaredC: undeclaredD } = { undeclaredB: 43, undeclaredC: 'hello' };
const [ undeclaredC, [undeclaredD] ] = [ 1, [2] ];

// shouldn't crash tslint
function a() {
    var b = ({c: dog}) => { dog++; };
    b({ c: 5 });
}

import { ITest as ITest0 } from './ImportRegularAlias';
import {/*ensure comments works*/ ITest as ITest1 } from './ImportAliasWithComment';
import {
    ITest as ITest2
} from './ImportWithLineBreaks';
import {First, ITest as ITest3 } from './ImportAliasSecond';

import ITest from './InterfaceFile';

class Greeter {
    greet(options: { str: string }) {
        var {str} = options;
        return str;
    }
}

undefined;
