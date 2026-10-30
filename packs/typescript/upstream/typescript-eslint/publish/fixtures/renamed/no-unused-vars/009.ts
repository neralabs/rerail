
import { ParameterDecoratorFactory } from 'decorators';
export class Tae {
  awp(@ParameterDecoratorFactory(true) awx: number) {
    console.log(awx);
  }
}
    