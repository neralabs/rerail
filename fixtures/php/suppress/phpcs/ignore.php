<?php

var_dump(1);
// phpcs:ignore
var_dump(2);
var_dump(3); // phpcs:ignore
// phpcs:ignore Squiz.PHP.DiscouragedFunctions
var_dump(4);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions.Discouraged
var_dump(5);
// phpcs:ignore Squiz.PHP
var_dump(6);
// phpcs:ignore Squiz
var_dump(7);
// phpcs:ignore Generic.PHP.ForbiddenFunctions
var_dump(8);
// phpcs:ignore Generic.PHP.ForbiddenFunctions, Squiz.PHP.DiscouragedFunctions -- a reason
var_dump(9); $a = sizeof([]);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions -- TODO: a reason with a task
var_dump(10);
// @phpcs:ignore Squiz.PHP.DiscouragedFunctions
var_dump(11);
# phpcs:ignore Squiz.PHP.DiscouragedFunctions
var_dump(12);
/* phpcs:ignore Squiz.PHP.DiscouragedFunctions */
var_dump(13);
/** phpcs:ignore Squiz.PHP.DiscouragedFunctions */
var_dump(14);
/**
 * @phpcs:ignore Squiz.PHP.DiscouragedFunctions
 */
var_dump(15);
/*
 * phpcs:ignore Squiz.PHP.DiscouragedFunctions
 */
var_dump(16);
// PHPCS:IGNORE Squiz.PHP.DiscouragedFunctions
var_dump(17);
// phpcs:ignore squiz.php.discouragedfunctions
var_dump(18);
// text phpcs:ignore
var_dump(19);
//phpcs:ignore
var_dump(20);
/* phpcs:ignore */ var_dump(21);
var_dump(22); /* phpcs:ignore */ var_dump(23);
// phpcs:ignore

var_dump(24);
$s = '// phpcs:ignore';
var_dump(25);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions.Other
var_dump(26);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions
var_dump(27); $b = TRUE;
// phpcs:ignore Generic
$c = @sizeof([]); var_dump(28);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions,Generic.PHP.LowerCaseConstant
var_dump(29) && TRUE;
$d = 1; // TODO: a task phpcs:ignore
// TODO: a task before phpcs:ignore
var_dump(30);
$heredoc = <<<TEXT
// phpcs:ignore
TEXT;
var_dump(31);
/** @see phpcs:ignore */
var_dump(32);
/**
 * Text.
 * phpcs:ignore Squiz.PHP.DiscouragedFunctions */
var_dump(33);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions
// phpcs:ignore Generic.PHP.LowerCaseConstant
var_dump(34) && TRUE;
var_dump(35); // phpcs:ignore Squiz.PHP.DiscouragedFunctions
    // phpcs:ignore Squiz.PHP.DiscouragedFunctions
    var_dump(36);
// phpcs:ignore Squiz.PHP.DiscouragedFunctions.
var_dump(37);
// phpcs:ignorex
var_dump(38);
// phpcs:ignore -- why?
var_dump(39);
# phpcs:ignore -- why?
var_dump(40);
