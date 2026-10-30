
interface Test {
  ['f']: <T extends {}>(a: T, b: T) => T;
}
    