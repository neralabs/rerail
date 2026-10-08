<?php

// PHP compares a switch's cases in order: a case's body runs with every earlier case's value
// evaluated, and `default` with all of them (wherever it is written; PHPStan 2.2.16 reads a
// `default` written first as entered before any case, which
// `scope::php::tests::a_switch_enters_default_after_every_case_value` pins against PHP).
function cases(int $n): void
{
    switch ($n) {
        case $first = 1:
            echo $second;
        case $second = 2:
            echo $first;
            echo $second;
            break;
        case 3:
            echo $first;
            echo $second;
            $fallthrough = true;
        case 4:
            echo $fallthrough;
            echo $first;
    }
    echo $first;
}

function noMatch(int $n): void
{
    switch ($n) {
        // A comment between the cases is no default.
        case $only = 1:
            break;
    }
    echo $only;
}

// The HTTP stream wrapper defines $http_response_header in the scope of the call that opens a
// URL; PHPStan defines it after fopen() and file_get_contents().
function headers(string $url): void
{
    echo count($http_response_header);
    file_get_contents($url);
    echo count($http_response_header);
}

function headersFromFile(string $url): void
{
    file($url);
    echo count($http_response_header);
}
