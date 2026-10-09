namespace N {
    export type T = number;
    export const x: N.T = 0;

    export namespace M {
        export type U = N.T;
        export const y: M.U = 0;
        export const z: N.M.U = 0;
        export const a = N.x;
        export const b = M.y;
        export const c = N.M.z;
    }

    #if typescript >= 3.8.0
    export class C {
        #p: string;

        constructor() {
            this.#p = 'C!';
        }

        getP() {
            return this.#p;
        }
    }
    #endif
}

namespace A.B.C1 {
    export const D = 7;
}

namespace A.B.C2 {
    const copy = A.B.C1.D;
}

import * as B from "./b.test";
declare module "./b.test" {
    const x: B.T;
}

namespace X {
    export type T = number;
    namespace Y {
        type T = string;
        // This qualifier *is* necessary since 'X.T' is shadowed.
        const x: X.T = 0;
    }
}

enum E {
    A,
    B = E.A
}

