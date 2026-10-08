<?php

// @phpstan-ignore echo.nonString (reason (nested))
echo $standalone;
echo $trailing; // @phpstan-ignore echo.nonString
# @phpstan-ignore echo.nonString
echo $hash;
/* @phpstan-ignore echo.nonString */
echo $block;
/** @phpstan-ignore echo.nonString */
echo $doc;
/* multi-line
 * @phpstan-ignore echo.nonString (reason (nested))
 */
echo $multiline;
// @phpstan-ignore-line
echo $line;
echo $lineTrailing; // @phpstan-ignore-line
# @phpstan-ignore-next-line
echo $nextHash;
/* @phpstan-ignore-next-line */
echo $nextBlock;
/** @phpstan-ignore-next-line */
echo $nextDoc;
$literal = '@phpstan-ignore-line';
echo $string;
// @phpstan-ignore
 echo $bare;
// @phpstan-ignore echo.other
echo $wrong;

// prefix text @phpstan-ignore echo.nonString
echo $prefix;
// @phpstan-ignore echo.nonString, variable.undefined (two identifiers)
echo $spaced;
// @phpstan-ignore echo.nonString,variable.undefined (no list space)
echo $compact;
/**
 * @var mixed $tagged
 * @phpstan-ignore echo.nonString
 */
echo $tagged;
echo $beforeBlock; /* @phpstan-ignore echo.nonString */ echo $afterBlock;
// @phpstan-ignore echo.nonString
echo (
    $multilineStatement
);
// @phpstan-ignore echo.nonString

echo $blank;
// @phpstan-ignore echo.nonString
// an intervening comment
echo $intervening;
// @phpstan-ignore-next-line

echo $nextBlank;
/*
 * @phpstan-ignore-line
 */
echo $multilineLine;
// @PHPStan-ignore echo.nonString
echo $uppercase;
// @phpstan-ignore echo/nonString
echo $invalidIdentifier;
$double = "@phpstan-ignore echo.nonString";
echo $doubleQuoted;
$heredoc = <<<TEXT
// @phpstan-ignore echo.nonString
TEXT;
echo $heredoc;
$nowdoc = <<<'TEXT'
/* @phpstan-ignore echo.nonString */
TEXT;
echo $nowdoc;
// @phpstan-ignore echo.nonString
echo $twoErrorsOneLine, $twoErrorsToo;
