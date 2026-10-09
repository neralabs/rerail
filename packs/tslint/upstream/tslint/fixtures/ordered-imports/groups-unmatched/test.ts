#!/usr/bin/env node
/* Test to check that all unmatched imports end up in unmatched group at end.*/

import {app_f} from 'app/foo';

import {x} from '@pkg/foo';

import {app_b} from 'app/bar';

import {a} from '@pkg/bar';

import {xbar} from '../xbar';

import {foo} from 'foo';
import {b} from './ybar';

export class Test {}
