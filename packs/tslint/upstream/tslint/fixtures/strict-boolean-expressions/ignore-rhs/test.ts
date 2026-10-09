class C { }
enum MyEnum {
	A, B, C
}
let anyType: {};
let boolType: boolean;
let boolType2: boolean;
let bwrapType: Boolean;
let numType = 9;
let strType = "string";
let objType = new Object();
let classType = new C();
let enumType = MyEnum.A;
let boolFn = () => { return true; };
let strFn = () => { return strType; };
let numFn = () => { return numType; };
let boolExpr = (strType !== undefined);

/*** Binary Expressions ***/
/*** Invalid Boolean Expressions ***/
classType && boolType;
anyType && boolType;
numType && boolType;
boolType && strType;

boolType && objType && enumType;

boolType && objType || enumType;

boolType || objType && enumType;

boolType || objType || enumType;

bwrapType && boolType;

boolType || classType;
boolType || anyType;
boolType || numType;

strType || boolType;

bwrapType || boolType;

objType || boolType || enumType;

boolExpr && strType;

numType || boolExpr;

numType && boolExpr || strType;

bwrapType || boolExpr && bwrapType;

let bool1:boolean = true
let bool2:boolean = true
let obj:{}|undefined = {}

bool1 && obj && bool2
bool1 && bool2 && obj
boolExpr && numType || strType;
boolExpr || numType || strType;
boolExpr || numType && strType;
boolExpr && numType && strType;
boolExpr || boolType && strType;
boolExpr || boolType || strType;
boolExpr && boolType || strType;
