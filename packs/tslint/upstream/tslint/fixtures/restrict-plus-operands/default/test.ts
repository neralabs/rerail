// aliases for number
type MyNumber = number;
type MyString = string;
interface NumberStringPair {
    first: MyNumber,
    second: MyString
}

var x = 5;
var y = "10";
var z = 8.2;
var w = "6.5";
var pair: NumberStringPair = {
    first: 5,
    second: "10"
};

// bad
var bad1  = 5 + "10";
var bad2  = [] + 5;
var bad3  = [] + {};
var bad4  = [] + [];
var bad4  = 5 + [];
var bad5  = "5" + {};
var bad6  = 5.5 + "5";
var bad7  = "5.5" + 5;
var bad8  = x + y;
var bad9  = y + x;
var bad10 = x + {};
var bad11 = [] + y;
var bad12 = pair.first + "10";
var bad13 = 5 + pair.second;
var bad14 = pair + pair;
var anyTyped: any = 5;
var bad15 = anyTyped + 12;

// good
var good1 = 5 + 10;
var good2 = "5.5" + "10";
var good3 = parseFloat("5.5", 10), + 10;
var good4 = x + z;
var good5 = y + w;

var good6 = pair.first + 10;
var good7 = pair.first + (10 as MyNumber);
var good8 = "5.5" + pair.second;
var good9 = ("5.5" as MyString) + pair.second;
const good10 = 'hello' + (someBoolean ? 'a' : 'b') + (() => someBoolean ? 'c' : 'd')() + 'e';

// don't check other binary expressions
const balls = true;
balls === true;
