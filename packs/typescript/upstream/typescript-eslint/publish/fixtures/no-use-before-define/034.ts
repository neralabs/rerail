
global.foo = true;

declare global {
  namespace NodeJS {
    interface Global {
      foo?: boolean;
    }
  }
}
    