function() {
    try {
    } finally {
        return;
    }
}

function() {
    try {
    } catch {
    } finally {
        return 1;
    }
}

function() {
    try {
    } catch {
    } finally {
        {
            return 1;
        }
    }
}

function() {
    try {
    } catch {
    } finally {
        for (let i = 0; i < 5; ++i) {
            if (i % 2 === 0) {
                return 1;
            }
        }
    }
}

function() {
    try {
    } catch {
    } finally {
        let i = 1;
        while (i < 5) {
            if (i % 2 === 0) {
                return i;
            }

            ++i;
        }
    }
}

function() {
    try {
    } catch {
    } finally {
        let i = 1;
        do {
            if (i % 2 === 0) {
                return i;
            }

            ++i;
        } while (i < 5);
    }
}

function() {
    try {
        if (foo()) {
            try {
                bar();
            } catch {
            } finally {
                return 2;
            }
        }
    } catch {
    } finally {
        for (const i of [1, 2, 3, 4, 5]) {
            if (i % 2 === 0) {
                return 1;
            }
        }
    }
}

function foo() {
    try {
        try {
        } finally {
            return 1;
        }
    } catch {
    }
}

function foo() {
    try {
    } finally {
        throw "error";
    }
}

function foo() {
    for (const i of [1, 2, 3, 4, 5]) {
        try {
        } finally {
            continue;
        }
    }
}

function foo() {
    for (const i of [1, 2, 3, 4, 5]) {
        try {
        } finally {
            break;
        }
    }
}

function foo() {
    for (let i = 0; i < 5; ++i) {
        try {
        } finally {
            continue;
        }
    }
}

function foo() {
    do {
        try {
        } finally {
            continue;
        }
    } while (i > 0);
}

function foo() {
    while (i > 0) {
        try {
        } finally {
            continue;
        }
    }
}

function foo() {
    do {
        try {
        } finally {
            break;
        }
    } while (i > 0);
}

function foo() {
    while (i > 0) {
        try {
        } finally {
            break;
        }
    }
}

function foo() {
    switch (a) {
        case 1:
            try {
                return;
            } finally {
                break;
            }
            break;
        default:
            try {
            } finally {
                break;
            }
            break;
    }
}

function foo() {
    label:
    try {
    } finally {
        break label;
    }
}

function foo() {
    label:
    for (let i = 0; i < 4; ++i) {
        try {
        } finally {
            break label;
        }
    }
}

function foo() {
    outer:
    for (let i = 0; i < 4; ++i) {
        try {
        } finally {
            inner:
            for (let j = 0; j < 2; ++j) {
                break outer;
            }
        }
    }
}

function foo() {
    label:
    for (let i = 0; i < 4; ++i) {
        try {
        } finally {
            continue label;
        }
    }
}

function foo() {
    outer:
    for (let i = 0; i < 4; ++i) {
        try {
        } finally {
            inner:
            for (let j = 0; j < 2; ++j) {
                continue outer;
            }
        }
    }
}

// valid
function() {
    try {
    } finally {
        (function() {
            return 1;
        })();
    }
}

// valid
function() {
    try {
        foo();
    } finally {
        class C {
            bar() {
                return 1;
            }
        }
    }

    return 1;
}

// valid
function() {
    try {
    } catch {
    } finally {
        for (let i = 0; i < 5; ++i) {
            if (i % 2 === 0) {
                continue;
            }
        }
    }
}

// valid
function() {
    try {
    } catch {
    } finally {
        for (let i = 0; i < 5; ++i) {
            if (i % 2 === 0) {
                break;
            }
        }
    }
}

// valid
function() {
    try {
    } catch {
    } finally {
        switch (a) {
            case 1:
                break;
            default:
                break;
        }
    }
}

//valid
function() {
    try {
    } finally {
        label:
            break label;
    }
}

//valid
function() {
    try {
    } finally {
        outer:
        for (let i = 0; i < 2; ++i) {
            if (i % 2 === 0) {
                continue outer;
            }

            inner:
            for (let j = 0; j < 2; ++j) {
                if (j % 2 === 0) {
                    continue outer;
                }
            }
        }
    }
}

function() {
    try {
    } finally {
        function f() {
            return;
        }
    }
}

try {
} finally {
    break;
}

