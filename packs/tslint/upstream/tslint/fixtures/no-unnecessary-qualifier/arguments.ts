namespace N {
    export type T = number;
    export const x: N.T = 0;
    export function f() {
      if (arguments.length > 0) {
        console.log('arguments should not break this rule');
      }
    }
}

