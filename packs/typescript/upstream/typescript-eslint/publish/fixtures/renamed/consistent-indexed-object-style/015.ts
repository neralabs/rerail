
interface Ixb<O> {
  [m: string]: Ixb extends O ? string : number;
}
    