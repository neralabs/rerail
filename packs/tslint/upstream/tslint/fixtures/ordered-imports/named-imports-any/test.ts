// Named imports do not need to be alphabetized.
import {A, B} from 'foo';
import {B, A} from 'foo'; // should not fail

// Case is irrelevant for named import ordering.
import {A, b, C} from 'zfoo';
import {bz, A, C} from 'foo'; // should not fail

// Import sources should be alphabetized.
import * as bar from 'bar';
import * as foo from 'foo';

import * as abc from 'abc';
import * as foo from 'foo';
import * as bar from 'bar'; // failure

// Case is irrelevant for source import ordering.
import {A, B} from 'Bar';
import {A, B} from 'baz';
import {A, B} from 'Foo'; // should not fail

// Other styles of import statements.
import someDefault, {nameA, nameBReallyLong as anotherName} from "./wherever";
import someDefault from "module";
import "something";

// contains ImportEqualsDeclaration
import c = require('./c');
import * as a from './a';
import b from './b';

import * as a from './a';
import c = require('./c');
import b from './b';

import c = require('./c');
import b from './b';
import * as a from './a';

import d = require('./d');
import c = require('./c');

