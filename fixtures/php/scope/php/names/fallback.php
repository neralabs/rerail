<?php
// Unqualified functions and constants in a namespace: this file's declarations, PHP's builtins,
// and names neither declares.

namespace App;

function exec(string $command): string
{
    return $command;
}

function helper(): void
{
    function nested_helper(): void
    {
    }
    if (true) {
        function conditional_helper(): void
        {
        }
    }
}

const APP_NAME = 'app';
const FIRST = 1, SECOND = 2;
define('App\DEFINED', 1);
define("APP\\DEFINED_UPPER", 2);
\define('App\LEADING', 3);
define('Other\ELSEWHERE', 4);
define('GLOBAL_DEFINED', 5);
define($name, 6);
define('App\\' . 'CONCAT', 7);
define(value: 8, constant_name: 'App\NAMED');

exec('ls');
EXEC('ls');
helper();
nested_helper();
conditional_helper();
system('ls');
shell_exec('ls');
passthru('ls');
STRLEN('abc');
missing_function();
Closure::fromCallable('exec');

echo APP_NAME, FIRST, SECOND, DEFINED, DEFINED_UPPER, LEADING, ELSEWHERE, GLOBAL_DEFINED, CONCAT;
echo PHP_EOL, php_eol, E_ALL, M_PI, DIRECTORY_SEPARATOR, PHP_INT_MAX, SORT_STRING;
echo MISSING_CONSTANT, app_name, NAMED;
echo true, TRUE, false, null, Null;

class Runner
{
    public function exec(): void
    {
        exec('method');
        $this->exec();
        static::exec();
    }
}

namespace App\Sub;

exec('ls');
helper();
echo APP_NAME, DEFINED, PHP_EOL;

function exec(): void
{
}
