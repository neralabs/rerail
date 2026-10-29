// eslint-disable-next-line import/no-extraneous-dependencies
import 'jest-preset-angular/setup-jest';

import { webcrypto } from 'node:crypto';

Object.defineProperty(window, 'crypto', {
  get() {
    return webcrypto;
  },
});

global.EventSource = class {} as any;
