
async function promiseFunc() {
    return false;
}

class CustomPromise<T = any> {}
const customPromise = new CustomPromise<any>();

// Custom promise
if (customPromise) {}

normalExpression = customPromise;
const stringLiteral = "text" + customPromise;

const globalUnaryExpression = !!customPromise;

const globalBinaryExpression = "text" && customPromise;

customPromise && console.log("topLevelBinaryExpression");

function union(promiseOrUndefined: CustomPromise | undefined, promiseOrFalse: CustomPromise | false, promiseOrNull: CustomPromise | null) {
    if (promiseOrUndefined || promiseOrFalse || promiseOrNull) {
        return;
    }
}

function normalUrCustomPromise(onlyPromise: CustomPromise | Promise<any>, promiseOrNull: CustomPromise | Promise<any> | null) {
    if (onlyPromise && promiseOrNull) { }
}

