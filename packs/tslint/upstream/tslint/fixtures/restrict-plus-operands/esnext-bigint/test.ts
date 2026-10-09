type MyNumber = number;
type MyString = string;
type MyBigInt = bigint;
interface NumberStringBigint {
    first: MyNumber,
    second: MyString,
    third: MyBigInt
}

var x = 5;
var y = "10";
var bigintVar = BigInt(200);
var anyVar: any = 200;
var pair: NumberStringBigint = {
    first: 5,	    first: 5,
    second: "10"	    second: "10",
    third: BigInt(100),
};

const bigIntPassA = BigInt(1) + BigInt(2);
const bigIntPassB = BigInt(1) + 100n;
const bigIntFailA = BigInt(1) + 2;
const bigIntFailB = BigInt(1) + "failureString";

const bigIntFailC = bigintVar + x;
const bigIntFailD = y + bigintVar;
const bigIntFailE = bigintVar + anyVar;

const bigIntFailF = pair.first + pair.third;
const bigIntFailG = pair.third + pair.second;
const bigIntFailH = bigintVar + [];
const bigIntFailI = bigintVar + {};
