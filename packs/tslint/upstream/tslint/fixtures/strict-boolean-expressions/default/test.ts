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

/*** Valid Boolean Expressions ***/
boolType && boolType2;
boolExpr || boolType;
(numType > 0) && boolFn();
(strType !== "bool") && boolExpr;
(numType > 0) && (strType !== "bool");
(strType !== undefined) || (numType < 0);

/*** ConditionalExpression ***/
/*** Invalid ***/
strType ? strType : numType;
numType ? numType : numFn();
objType ? objType : boolExpr;
classType ? strType : undefined;
bwrapType ? 1 : 0;
enumType ? 0 : 1;

/*** Valid ***/
boolFn() ? numType : strType;
boolType ? strType : undefined;

/*** PrefixUnary Expressions ***/
/*** Invalid ***/
!!numType;
!strType;
!objType;
!enumType;
!!classType;
!bwrapType;
!!undefined;

/*** Valid ***/
!!boolFn();
!boolExpr;
!!boolType;

/*** If Statement ***/
/*** Invalid ***/
if (numType) { }
if (objType) { }
if (strType) { }
if (bwrapType) { }
if (strFn()) { }
if (MyEnum.A) { }
if (classType) { }

/*** Valid ***/
if (boolFn()) { }
if (boolExpr) { }
if (boolType) { }

/*** While Statement ***/
/*** Invalid ***/
while (numType) { break; }
while (objType) { break; }
while (strType) { break; }
while (strFn()) { break; }
while (bwrapType) { break; }
while (MyEnum.A) { break; }
while (classType) { break; }

/*** Valid ***/
while (boolFn()) { break; }
while (boolExpr) { break; }
while (boolType) { break; }

/*** Do Statement ***/
/*** Invalid ***/
do { break; } while (numType);
do { break; } while (objType);
do { break; } while (strType);
do { break; } while (bwrapType);
do { break; } while (strFn());
do { break; } while (MyEnum.A);
do { break; } while (classType);

/*** Valid ***/
do { break; } while (boolFn());
do { break; } while (boolExpr);
do { break; } while (boolType);

/*** For Statement ***/
/*** Invalid ***/
for (let j = 0; j; j++) { break; }
/*** Valid ***/
for (let j = 0; j > numType; j++) { break; }

// Allow always-truthy or always-falsy in the case of 'true' and 'false'
while (true) {}
if (false) {}
// Do *not* allow 'true' in a union
declare var trueOrDate: true | Date;
if (trueOrDate) {}

declare var a: any;
// Allow 'any'
if (a) {}

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

  bool1 && obj && bool2

  obj && bool2 && obj

  obj || bool2 || obj

  obj || bool2 && obj

