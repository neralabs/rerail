#!/usr/bin/env node
/* Test a more complex set of split up groups. */

// comment outside of imports
import {app_f} from 'app/foo';
import {x} from '@pkg/foo';

import {app_b} from 'app/bar';
// comment pkg/bar
import {a} from '@pkg/bar';

import {xbar} from '../xbar';

import {bar} from '../bar';

import {foo, afoo} from 'foo';

import x = require('y');
x.fnCall();

import './baz'; // required
import './baa';


export class Test {}
