// Other styles of import statements.
import someDefault, {nameA, nameBReallyLong as anotherName} from "./wherever";
import someDefault from "module";
import "something";

// contains relative path & ImportEqualsDeclaration
import c = require('./baz/c');
import a from './foo/a';
import b from './bar/b';

// contains relative path & ImportEqualsDeclaration
import c = require('./baz/c');
import a from '../../foo/a';
import b from './bar/b';

// contains scoped import
import b = require('foo/b');
import a from 'foo/a';
import c from 'foo/c';
