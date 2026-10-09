// Test each function type.
const validFunctionExpression = function() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
}

const invalidFunctionExpression = function() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    const condition3 = true ? "true" : "false";
}


function validFunctionDeclaration() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
}

function invalidFunctionDeclaration() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    const condition3 = true ? "true" : "false";
}

const validArrowFunction = () => {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
}

const invalidArrowFunction = () => {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    const condition3 = true ? "true" : "false";
}

class MyClass {
    get validGet(): string {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
        return "test";
    }

    get invalidGet(): string {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
        const condition3 = true ? "true" : "false";
        return "test";
    }

    set validSet(newValue: string) {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
    }

    set invalidSet(newValue: string) {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
        const condition3 = true ? "true" : "false";
    }

    validMethod() {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
    }

    invalidMethod() {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
        const condition3 = true ? "true" : "false";
    }
}

class validConstructor {
    constructor() {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
    }
}

class invalidConstructor {
    constructor() {
        const condition1 = true ? "true" : "false";
        const condition2 = true ? "true" : "false";
        const condition3 = true ? "true" : "false";
    }
}

// Test each remaining expression type.
function validBinaryExpression() {
    return first || second && third
}

function invalidBinaryExpression() {
    return first || second && third || fourth;
}

function validSwitch() {
    switch(5) {
        case 0:
        case 2:
            return "even";
        case 1:
        case 3:
            return "odd";
        default:
            return -1;
    }
}

function invalidSwitch() {
    switch(5) {
        case 0:
            return 0;
        case 1:
            return 1;
        case 2:
            return 2;
        default:
            return -1;
    }
}

function validCatch() {
    try { }
    catch(error) { }
    try { }
    catch(error) { }
    try { }
    finally { }
}

function invalidCatch() {
    try { }
    catch(error) { }
    try { }
    catch(error) { }
    try { }
    catch(error) { }
    try { }
    finally { }
}

function validDo() {
    do { }
    while (false)
    do { }
    while (false)
}

function invalidDo() {
    do { }
    while (false)
    do { }
    while (false)
    do { }
    while (false)
}

function validFor() {
    for(;;) { }
    for(;;) { }
}

function invalidFor() {
    for(;;) { }
    for(;;) { }
    for(;;) { }
}

function validForIn() {
    for(let i in [0,1,2]) { }
    for(let i in [0,1,2]) { }
}

function invalidForIn() {
    for(let i in [0,1,2]) { }
    for(let i in [0,1,2]) { }
    for(let i in [0,1,2]) { }
}

function validForOf() {
    for(let i of [0,1,2]) { }
    for(let i of [0,1,2]) { }
}

function invalidForOf() {
    for(let i of [0,1,2]) { }
    for(let i of [0,1,2]) { }
    for(let i of [0,1,2]) { }
}

let variable = 1;

function validIf() {
    if(variable) { }
    if(variable) { } else { }
}

function invalidIf() {
    if(variable) { }
    if(variable) { } else { }
    if(variable) { }
}

function validWhile() {
    while(false) { }
    while(false) { }
}

function invalidWhile() {
    while(false) { }
    while(false) { }
    while(false) { }
}

// Test that the rule behaves correctly for nested functions.

// Valid outer function, invalid inner function.
function outer1() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    function inner1() {
        const condition3 = true ? "true" : "false";
        const condition4 = true ? "true" : "false";
        const condition5 = true ? "true" : "false";
    }
}

// Invalid outer function, valid inner function.
function outer2() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    const condition3 = true ? "true" : "false";
    function inner2() {
        const condition4 = true ? "true" : "false";
        const condition5 = true ? "true" : "false";
    }
}

// Both valid.
function outer3() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    function inner3() {
        const condition3 = true ? "true" : "false";
        const condition4 = true ? "true" : "false";
    }
}

// Both invalid.
function outer4() {
    const condition1 = true ? "true" : "false";
    const condition2 = true ? "true" : "false";
    const condition3 = true ? "true" : "false";
    function inner4() {
        const condition4 = true ? "true" : "false";
        const condition5 = true ? "true" : "false";
        const condition6 = true ? "true" : "false";
    }
}
