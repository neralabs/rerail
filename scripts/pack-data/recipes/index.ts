/**
 * Every tool-pack's derivation recipe, one module per pack (scripts/pack-data/recipes/<pack>.ts). Every tool directory
 * under packs/<id>/upstream/ with pack test data has one.
 */
import type { DeriveRecipe } from "../context.js";
import { recipes as blade } from "./blade.js";
import { recipes as complexity } from "./complexity.js";
import { recipes as javascript } from "./javascript.js";
import { recipes as next } from "./next.js";
import { recipes as node } from "./node.js";
import { recipes as php } from "./php.js";
import { recipes as python } from "./python.js";
import { recipes as react } from "./react.js";
import { recipes as rust } from "./rust.js";
import { recipes as sql } from "./sql.js";
import { recipes as typescript } from "./typescript.js";
import { recipes as vue } from "./vue.js";

export const RECIPES: readonly DeriveRecipe[] = [...blade, ...complexity, ...javascript, ...next, ...node, ...php, ...python, ...react, ...rust, ...sql, ...typescript, ...vue];
