/** @typedef {number} T */

/** @function
 *  @this
 */
function f() {}

/** @type number */
const x = 0;

/**
 * @class
 * @this {}
 * @param {number} x Is a number
 * @param y
 * @param {number} z
 * @returns {number}
 */
declare function g(x: number, y: number, z: number): number;

/**
 * @this {SomeClass}
 */
declare function h();

/**
 * @author John Doe <john.doe@example.com>
 * @param x Useful comment
 * @returns Useful comment
 */
declare function i(x: number): number;

/**
 * @template T, U
 * @template V Some additional information
 */
declare function j<T, U, V>(x: T, y: V): U;

