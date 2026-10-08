import fs from 'fs';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import lodash from 'lodash';
import local from './fs.js';
export * from 'node:path';
const sqlite = await import('node:sqlite');
const nope = await import('node:nope');
