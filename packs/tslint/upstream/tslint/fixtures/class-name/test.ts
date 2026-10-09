class ValidClassName {

}

class invalidClassName {
}

class Another_Invalid_Class_Name {
}

export default class {
    // should not fail
}

// anonymous class expression
var foo = class {};
var bar = class invalidName {}

interface someInterface {}
interface SomeInterface {}

