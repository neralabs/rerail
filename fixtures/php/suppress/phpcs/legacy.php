<?php

var_dump(1);
// @codingStandardsIgnoreLine
var_dump(2);
var_dump(3); // @codingStandardsIgnoreLine
// @codingStandardsIgnoreStart
var_dump(4); $a = TRUE;
// @codingStandardsIgnoreEnd
var_dump(5);
/**
 * @codingStandardsIgnoreLine
 */
var_dump(6);
/* @codingStandardsIgnoreLine */
var_dump(7);
// some text @codingStandardsIgnoreLine
var_dump(8);
// @codingStandardsIgnoreStart
var_dump(9);
var_dump(10); // @codingStandardsIgnoreEnd
var_dump(11);
// @codingstandardsignoreline
var_dump(12);
// @codingStandardsIgnoreStart
// phpcs:enable
var_dump(13);
// @codingStandardsIgnoreEnd
var_dump(14);
// phpcs:disable Squiz.PHP.DiscouragedFunctions
// @codingStandardsIgnoreStart
var_dump(15); $b = TRUE;
// @codingStandardsIgnoreEnd
var_dump(16); $c = TRUE;
// phpcs:enable
/** @codingStandardsIgnoreLine */
var_dump(17);
    // @codingStandardsIgnoreLine
    var_dump(18);
