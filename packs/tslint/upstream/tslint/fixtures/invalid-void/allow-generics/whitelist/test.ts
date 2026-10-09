type Allowed<T> = [T];
type AllowedVoid = Allowed<void>;

type Banned<T> = [T];
type BannedVoid = Banned<void>;

function takeVoid(thing: void) { }

