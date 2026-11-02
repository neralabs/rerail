
class Qgf<R> {
  value: R;
}
class Gun<R> {
  qgf = Qgf<R>;
}
new Gun();
    