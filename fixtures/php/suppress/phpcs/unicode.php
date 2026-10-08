<?php

var_dump('éx'); // phpcs:ignore
var_dump('éx', TRUE); // phpcs:ignore Squiz.PHP.DiscouragedFunctions
var_dump(1); echo '日x'; # phpcs:ignore
$s = '日本'; var_dump($s, FALSE); /* phpcs:ignore Generic.PHP.LowerCaseConstant */
var_dump('éx', NULL);
