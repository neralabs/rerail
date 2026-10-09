switch (value) {
    case 1: break;
    default: break;
}

export { SingleItem };

export { FirstItem, SecondItem };

export { Item as aliasForItem };

export { ItemFromImport } from 'module';

export * from './relativeModule';

export namespace SomeNamespace { }

export import ItemAlias = someNamespace.Item;

export const default = 'VALID';

export = 'VALID';

export default 'INVALID';

export default { }

export default class { }

export default class Test { }

export default abstract class { }

export default function() { }
       
export default function Test() { }


