// ---------------------------------------------------------------------------------------------------------------------
// Module with side effect import
// ---------------------------------------------------------------------------------------------------------------------
// good
import "styles.css";

// bad
import"styles.css";
import  "styles.css";
import   "styles.css";

import
"styles.css";
// ---------------------------------------------------------------------------------------------------------------------
// Namespace import
// ---------------------------------------------------------------------------------------------------------------------
// good
import * as ts from "typescript";

// bad
import*as ts from "typescript";
import *as ts from "typescript";
import *  as ts from "typescript";
import *   as ts from "typescript";
import* as ts from "typescript";
import * as ts from"typescript";
import  * as ts from "typescript";
import    * as ts from "typescript";
import * as ts from  "typescript";
import
* as ts from "typescript";

import *
as ts from "typescript";

import * as
ts from "typescript";

import * as ts
from "typescript";


// ---------------------------------------------------------------------------------------------------------------------
// Single-line import
// ---------------------------------------------------------------------------------------------------------------------
// good
import {Expression} from "typescript";

// bad
import{Expression}from"typescript";
import {Expression}from"typescript";
import {Expression} from"typescript";
import  {Expression} from "typescript";
import {Expression}  from "typescript";
import {Expression} from  "typescript";
import  {Expression} from "typescript";
import   {Expression} from "typescript";

import
{Expression} from "typescript";

import {Expression}
from "typescript";

import {Expression} from
"typescript";

// ---------------------------------------------------------------------------------------------------------------------
// Multi-line import
// ---------------------------------------------------------------------------------------------------------------------
// good
import {
    Expression
} from "typescript";

// bad
import{
    Expression
} from "typescript";

import {Expression
}from "typescript";

import {
    Expression
} from"typescript";

import  {
    Expression
} from "typescript";

import  {
    Expression
} from "typescript";

import {
    Expression
}
from "typescript";

import {
    Expression
} from
"typescript";

import
{
    Expression
} from "typescript"
