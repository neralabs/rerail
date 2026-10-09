type Generic<T> = [T];
type GenericVoid = Generic<void>;

function takeVoid(thing: void) { }

let voidPromise: Promise<void> = new Promise<void>(() => {});

let voidMap: Map<string, void> = new Map<string, void>();

