// Named imports should be alphabetized.
import {A, B} from 'foo';
import {B, A} from 'foo'; // failure

// Lowercase comes before uppercase.
import {A, b, C} from 'zfoo';
import {bz, A, C} from 'foo'; // failure

// Import sources should be alphabetized.
import * as bar from 'bar';
import * as foo from 'foo';

import * as abc from 'abc';
import * as foo from 'foo';
import * as bar from 'bar'; // failure

// Lowercase comes before uppercase
import {A, B} from 'Bar';
import {A, B} from 'baz';
import {A, B} from 'Foo';

// Other styles of import statements.
import someDefault, {nameA, nameBReallyLong as anotherName} from "./wherever";
import someDefault from "module";
import "something";

// contains ImportEqualsDeclaration
import c = require('./c');
import * as a from './a';
import b from './b';

