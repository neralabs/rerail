var validName1 = "hi";
var VALIDNAME2 = "there";
var valid_name3 = "tslint";
var Invalid_name1 = " ";  // failure
var InvalidName2 = " ";  // failure

class Test {
    private valid_name = " ";
    private Invalid_name1 = "how"; // failure
    private _optionallyValid = "are"; // sometimes a failure
}

function test() {
    () => {
        var valid_name = " ";
        var InVaLiDnAmE4 = "you"; // failure
    };
}

declare var DeclaresAreValid: any;

export function functionWithInvalidParamNames (ok_name, BadName) { // 1 failure
}

let { foo, bar } = { foo: 1, bar: 2 };
let [ InvalidFoo, invalid_bar, ...invalid_baz ] =  [1, 2, 3, 4]; // 1 failure

export function anotherFunctionWithInvalidParamNames ([first_element, SecondElement]) { // 1 failure
    //
}

export function functionWithInvalidSpread(InvalidArg: ...number) { // 1 failure
  //
}

let optionallyValid_ = "bar";
let _$httpBackend_ = "leading and trailing";

// Aliases.
class X {
  ValidAlias = ValidAlias;
}

var ValidAlias = some.ValidAlias;
var ValidAlias = some.InValidAlias;

var someObject = {MoreValidAlias: 1};
var {MoreValidAlias: localVar} = someObject;
var {MoreValidAlias: local_var} = someObject;
var {MoreValidAlias: LocalVar} = someObject;
var {MoreValidAlias} = someObject;
