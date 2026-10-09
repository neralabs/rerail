interface I { }

interface J extends I { }

interface K { x: number; }

interface L extends J, K {} // extending more than one interface is ok, as it can be used instead of intersection types

interface M extends {} // don't crash on empty extends list

interface N extends Promise<string|number> {}
