<?php
// Writes php-syntactic.expected.json: for every case in php-syntactic.cases.json, the argument
// `arguments` with `mode: "syntactic"` finds for each slot of the case's signature in its call,
// as Ruff's `find_argument` reads a call, over nikic/PHP-Parser 5.9.0's `args`: the first named
// argument of the slot's name, else the positional argument at the slot's position among those
// before the first unpacking (`...`). `slots` holds each found argument's value as source text,
// `null` where none is found; `callable` marks a first-class callable (`f(...)`), which is no call.
// Run offline, not in CI, with PHP-Parser installed by Composer:
//   composer require --working-dir=/tmp/rerail-php-tools nikic/php-parser:5.9.0
//   RERAIL_PHP_TOOLS=/tmp/rerail-php-tools /opt/homebrew/Cellar/php/8.5.11/bin/php generate_php_syntactic.php

$tools = getenv('RERAIL_PHP_TOOLS') ?: '/tmp/rerail-php-tools';
require "$tools/vendor/autoload.php";

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;

const PARSER_VERSION = '5.9.0';

$installed = \Composer\InstalledVersions::getPrettyVersion('nikic/php-parser');
if (ltrim($installed, 'v') !== PARSER_VERSION) {
    fwrite(STDERR, "nikic/php-parser $installed installed; this oracle pins " . PARSER_VERSION . "\n");
    exit(1);
}

$parser = (new ParserFactory())->createForVersion(PhpVersion::fromString('8.5'));
$here = __DIR__;
$cases = json_decode(file_get_contents("$here/php-syntactic.cases.json"), true, flags: JSON_THROW_ON_ERROR);
$out = ['php' => PHP_VERSION, 'phpParser' => PARSER_VERSION, 'cases' => []];
foreach ($cases['cases'] as $case) {
    $source = "<?php {$case['call']};";
    $call = $parser->parse($source)[0]->expr;
    if (!$call instanceof Expr\CallLike) {
        throw new RuntimeException("`{$case['call']}` is no call");
    }
    if ($call->isFirstClassCallable()) {
        $out['cases'][] = $case + ['callable' => true];
        continue;
    }
    $positional = [];
    $named = [];
    $unpacked = false;
    foreach ($call->getArgs() as $argument) {
        if ($argument->unpack) {
            $unpacked = true;
        } elseif ($argument->name !== null) {
            $named[] = $argument;
        } elseif (!$unpacked) {
            $positional[] = $argument;
        }
    }
    $text = fn (Node $node) => substr($source, $node->getStartFilePos(), $node->getEndFilePos() - $node->getStartFilePos() + 1);
    $slots = [];
    foreach ($case['signature'] as $position => $slot) {
        $found = null;
        foreach ($named as $argument) {
            if ($argument->name->toString() === $slot) {
                $found = $argument;
                break;
            }
        }
        $found ??= $positional[$position] ?? null;
        $slots[] = $found === null ? null : $text($found->value);
    }
    $out['cases'][] = $case + ['slots' => $slots];
}
file_put_contents("$here/php-syntactic.expected.json", json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE) . "\n");
