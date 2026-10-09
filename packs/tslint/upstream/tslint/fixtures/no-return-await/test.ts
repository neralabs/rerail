async function foo() {
    return await x;
}
(async () => await x);

async function allowTypeAssertion() {
    return (await x)!;
    return (await x as number);
    return <string> await x;
}
async function allowedInsideTryCatch() {
    try {
        return await x;
    } catch (e) {
        // handle error
        return await x; // not allowed in catch when there is no finally
    }
}
async function allowedInsideTryFinally() {
    try {
        return await x;
    } finally {
        // do cleanup, close connection for example
        return await x; // not allowed in finally at all
    }
}
async function allowedInsideCatchFinally() {
    try {
        return await x;
    } catch (e) {
        return await x;
    } finally {
        return await x;
    }
}
async function nestedTryStatements() {
    try {
        try {
            return await x;
        } catch (e) {
            return await x;
        } finally {
            return await foobar;
        }
    } catch(e) {
        try {
            return await x;
        } catch (e) {
            return await x;
        } finally {
            return await foobar;
        }
    } finally {
        try {
            return await x;
        } catch (e) {
            return await x;
        } finally {
            return await x;
        }
    }
}

async function handleParens() {
    return (await x);
}
async function handleBinaryExpression() {
    return await foo || await x;
    return await foo && await x;
    return await foo, await x;
    return await foo + await bar;
}
async function handleConditionalExpression() {
    return await foo ? await bar : await baz;
}

throw await x;
await x;
return await x;

