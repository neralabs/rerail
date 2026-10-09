#!/usr/bin/env node
/* Test the case where import should be grouped using default grouping. */

import {bar} from '../bar';

import {foo, afoo} from 'foo';

import './baz'; // required
import './baa';

import './caa';

import x = require('y');

export class Test {}
