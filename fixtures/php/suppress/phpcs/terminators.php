<?php

var_dump(1);
// phpcs:ignore
var_dump(2);
var_dump(3); // phpcs:ignore
/*
 * phpcs:ignore
 */
var_dump(4);
// phpcs:disable Squiz
var_dump(5);
// phpcs:enable
var_dump(6);
