
interface Csx<P> {
  [n: string]: P extends Csx ? string : number;
}
    