import * as Utils from "tslint-utils"

import TslintUtils from "tslint-utils"

import Bar, { Foo } from "tslint-misc"

import Bar, * as Foo from "tslint-misc"

import { default as Foo } from "tslint-misc"

import { default as foo, bar } from "tslint-misc"

import { bar, default as foo } from "tslint-misc"

import TslintUtils from "../tslint-utils"

import TslintUtils from "./tslint-utils"

import Bar, { Foo } from "../tslint-misc"

import Bar, * as Foo from "./tslint-misc"

import { default as Foo } from "../tslint-misc"

import { default as foo, bar } from "./tslint-misc"

import { bar, default as foo } from "../tslint-misc"

