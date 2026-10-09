export class Aaa {
    public bbb;

    public ccc() { }

    public get prop() { return this.bbb; }

    public set prop(value) { this.bbb = value; }

    [Symbol.iterator]() {}
}

export enum Ddd { }

export function Eee() { }

export interface Fff { }

export namespace Ggg { }

export type Hhh = any;

export const i = true;
