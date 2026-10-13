let id;
function foo() {
    if (typeof id !== 'undefined') {
        return;
    }
    id = setInterval(() => {}, 250);
}
foo();