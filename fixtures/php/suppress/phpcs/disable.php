<?php

var_dump(1);
// phpcs:disable
var_dump(2); $a = TRUE;
// phpcs:enable
var_dump(3);
// phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(4); $b = TRUE;
// phpcs:enable
var_dump(5);
// phpcs:disable Squiz.PHP.DiscouragedFunctions, Generic.PHP.LowerCaseConstant -- reason
var_dump(6); $c = TRUE;
// phpcs:enable Generic.PHP.LowerCaseConstant
var_dump(7); $d = TRUE;
// phpcs:enable Squiz.PHP.DiscouragedFunctions
var_dump(8);
// phpcs:disable
var_dump(9);
// phpcs:enable Squiz.PHP.DiscouragedFunctions
var_dump(10); $e = TRUE;
// phpcs:enable
// phpcs:disable Squiz
var_dump(11);
// phpcs:enable Squiz.PHP.DiscouragedFunctions
var_dump(12);
// phpcs:enable
// phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(13);
// phpcs:disable Generic.PHP.LowerCaseConstant
var_dump(14); $f = TRUE;
// phpcs:ignore Generic.PHP.NoSilencedErrors
$g = @TRUE;
// phpcs:enable
var_dump(15); // phpcs:disable
var_dump(16);
var_dump(17); // phpcs:enable
var_dump(18);
// phpcs:enable
// @phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(19);
/* phpcs:enable */
var_dump(20);
/**
 * @phpcs:disable Squiz.PHP.DiscouragedFunctions
 */
var_dump(21);
/**
 * @phpcs:enable
 */
var_dump(22);
// phpcs:enable Squiz.PHP.DiscouragedFunctions
var_dump(23);
// phpcs:disable Generic.PHP
$h = @sizeof([TRUE]); var_dump(24);
// phpcs:enable Generic.PHP.ForbiddenFunctions
$i = @sizeof([TRUE]); var_dump(25);
// phpcs:enable
var_dump(26);
// phpcs:disable
// phpcs:enable Squiz
var_dump(27); $j = TRUE;
// phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(28); $k = TRUE; // TODO: in a disabled range
// phpcs:enable
var_dump(29);
// phpcs:disable Squiz.PHP.DiscouragedFunctions.Discouraged
var_dump(30);
// phpcs:enable Squiz.PHP.DiscouragedFunctions
var_dump(31);
// phpcs:enable
// phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(32);
// phpcs:enable Squiz.PHP.DiscouragedFunctions.Discouraged
var_dump(33);
// phpcs:enable
// phpcs:disable Squiz.PHP.DiscouragedFunctions
// phpcs:ignore Generic.PHP.LowerCaseConstant
var_dump(34) && TRUE;
var_dump(35) && TRUE;
// phpcs:enable
// phpcs:set Generic.PHP.ForbiddenFunctions forbiddenFunctions[] sizeof=>count
var_dump(36);
// phpcs:disable Squiz,Generic.PHP.LowerCaseConstant
var_dump(37) && TRUE;
// phpcs:enable Squiz
var_dump(38) && TRUE;
// phpcs:disable Squiz.PHP.DiscouragedFunctions
var_dump(39);
