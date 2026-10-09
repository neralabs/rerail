
const expr = "someStr" === "someStr";
const expr = 123 === 123;
const someVar = 100;
const expr = someVar === someVar;

const someFunc = () => {
    if ("SomeStr" === "SomeStr") {
        return true;
    }
}

const someFunc = () => {
    if (100 === 100) {
        return true;
    }
}

const someFunc = () => {
    let someVar = 100;
    if (someVar === someVar) {
        return true;
    }
}

const someFunc = () => {
    if (1 !== 1) {
        return true;
    }
}

const someFunc = () => {
    if (1 > 1) {
        return true;
    }
}

const someFunc = () => {
    const someVar = 123;
    if (someVar < someVar) {
        return true;
    }
}

const someFunc = () => {
    if ("str" == "str") {
        return true;
    }
}

const someFunc = () => {
    if (123 != 123) {
        return true;
    }
}

const someFunc = () => {
    if ("str" <= "str") {
        return true;
    }
}

const someFunc = () => {
    if ("str" >= "str") {
        return true;
    }
}

const someFunc = () => {
    const someVar = true;
    if (someVar || someVar) {
        return true;
    }
}

const someFunc = () => {
    const someVar = true;
    if (someVar && someVar) {
        return true;
    }
}

const someFunc = () => {
    const someObj = { "name" : "moses the great" };
    if (someObj.name === someObj.name) {
        return true;
    }
}

const someFunc = () => {
    const someObj = { "name" : "moses the great", "address" : "king's road 10" };
    if (someObj.name === someObj.address) {
        return true;
    }
}

const someFunc = () => {
    const objOne = { "name" : "moses the great" };
    const objTwo = { "name" : "moses the great" };
    if (objOne.name === objTwo.name) {
        return true;
    }
}

const someFunc = () => {
    if (true === true) {
        return true;
    }
}

const someFunc = () => {
    if (false === false) {
        return true;
    }
}

const someFunc = () => {
    if (true === false) {
        return true;
    }
}

const someFunc = () => {
    if (null === null) {
        return true;
    }
}

const someFunc = () => {
    if (null === false) {
        return true;
    }
}

const someVar1 = 3 + 3;
const someVar2 = 3 - 3;
const someVar3 = 3 * 3;
const someVar4 = 3 % 3;
const someVar5 = 3 / 3;
