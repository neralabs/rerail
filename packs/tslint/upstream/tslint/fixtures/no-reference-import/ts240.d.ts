/// <reference types="foo" />
/// <reference types="bar" />
/// <reference types="baz" />

import * as foo from "foo";

declare module "quux" {
    import * as baz from "baz";
}
