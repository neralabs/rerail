if (x === 1) {}
else if (x === 2) {} // Notice that we don't get *another* error here.
// Dangling `else` OK (this becomes `default:`)
else if (x === 3) {}
else {}

if (x === 1) {} else if (x === 2) {}

// Works with `||`
if (this === 1 || this === 2) {}

if (this === 1 || this === 2 || this === 3) {}


if (x === 1) {}
else if (x === 2) {}
else if (x === 3) {}

if (x === 1) {}
else if (x === 2) {}


// Default minumum of 2 cases.
if (x === 1) {} else {}

// Different variables, no failure
if (x === 1) {} else if (y === 2) {}

// Non-simple cases, no failure
if (x === f()) {} else if (x === g()) {}

// Allow properties
if (x.y.z === a.b) else if (x.y.z === c.d) {}

export enum ItemType {
    FIRST = "FIRST",
    SECOND = "SECOND",
    THIRD = "THIRD",
}

if (
    item.type === ItemType.FIRST ||
    item.type === ItemType.SECOND ||
    item.type === ItemType.THIRD
) {}

