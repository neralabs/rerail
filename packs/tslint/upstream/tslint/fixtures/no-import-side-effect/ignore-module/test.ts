// valid cases
import {Injectable, Component, Directive} from '@angular/core';
import {Observable} from 'rxjs/Observable';
import {Observable as Obs} from 'rxjs/Observable';
import * as _ from 'underscore';
import DefaultExport from 'module-with-default';
import './allow-side-effect';
import './styles.css';

// invalid cases
import 'rxjs/add/observable/of';
import 'rxjs/add/operator/mapTo';
import 'random-side-effect';

