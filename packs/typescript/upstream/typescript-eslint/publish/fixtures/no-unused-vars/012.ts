
import { configurable } from 'decorators';
export class A {
  @configurable(true) static prop1;

  @configurable(false)
  static prop2;
}
    