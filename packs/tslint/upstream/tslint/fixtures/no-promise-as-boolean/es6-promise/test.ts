
async function promiseFunc() {
    return false;
}

normalExpression = promiseFunc();
const stringLiteral = "text" + promiseFunc();

const globalUnaryExpression = !!promiseFunc();

const globalBinaryExpression = "text" && promiseFunc();

promiseFunc() && console.log("topLevelBinaryExpression");

function union(promiseOrUndefined: Promise<any> | undefined, promiseOrFalse: Promise<any> | false, promiseOrNull: Promise<any> | null) {
    if (promiseOrUndefined || promiseOrFalse || promiseOrNull) {
        return;
    }
};

function funky(maybePromise?: Promise<number>) {
    while (promiseFunc() && maybePromise) {

        const binaryExpression = (promiseFunc() && "1") || ("1" && !promiseFunc()) ? ("1" && promiseFunc() ? "1" : "1") : maybePromise;

        // For-loop
        for (let index = 0; promiseFunc(); index++) {

            // a non-promise
            if ("just some text" + promiseFunc()) {

            // Promise literal
            } else if (new Promise(() => { })) {

                // Nested Promise
                if (promiseFunc()) {
                }

            // Parenthesized Expression + Exclamation Tokens
            } else if (promiseFunc() && !!promiseFunc()) {
            }
        }
    }
}

async function waiter(custumerDecisionPromise: Promise<any>) {
    if (await custumerDecisionPromise) {
        console.log("Customer ready to take an order.")
    }
}

