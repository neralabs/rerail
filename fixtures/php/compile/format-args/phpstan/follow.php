<?php

declare(strict_types=1);

function variables(int $a, int $b, bool $c, string $p): void
{
    $two = '%s %s';
    echo sprintf($two, $a);
    echo sprintf($two, $a, $b);
    $one = '%s';
    echo sprintf($one, $a, $b);
    echo sprintf(($two), $a);
    $alias = $two;
    echo sprintf($alias, $a);
    printf($two, $a);
    echo vsprintf($two, [$a]);
    echo vsprintf($one, [$a, $b]);
    echo sprintf($two);
    $two = '%s';
    echo sprintf($two, $a, $b);
    echo sprintf($p, $a);
    echo sprintf($p, $a, $b);
}

function concatenation(int $a, int $b): void
{
    $cat = '%s' . ' %s';
    echo sprintf($cat, $a);
    echo sprintf('%s' . ' %s', $a);
    echo sprintf('%s' . ' %s', $a, $b);
    $one = '%s';
    echo sprintf($one . ' %d', $a);
    echo sprintf($one . ' %d', $a, $b);
    $n = 3;
    echo sprintf('%s ' . $n, $a, $b);
    echo sprintf((string) 12, $a);
    echo sprintf('%' . '1$s %' . '1$s', $a, $b);
    echo sprintf("%s\t" . '%s', $a);
}

function heredocs(int $a, int $b): void
{
    $heredoc = <<<EOT
        %s and %s
        EOT;
    echo sprintf($heredoc, $a);
    $nowdoc = <<<'EOT'
        %s
        EOT;
    echo sprintf($nowdoc, $a, $b);
    echo sprintf($nowdoc . <<<EOT
         %s
        EOT, $a);
}

function unknown(int $a, bool $c): void
{
    $g = $c ? '%s' : '%s %s';
    echo sprintf($g, $a);
    if ($c) {
        $h = '%s';
    } else {
        $h = '%s %s %s';
    }
    echo sprintf($h, $a);
    $i = '%s';
    $i .= ' %s';
    echo sprintf($i, $a);
    $invalid = '%y';
    echo sprintf($invalid, $a);
    $interpolated = "%s {$i}";
    echo sprintf($interpolated, $a);
    echo sprintf('%s' . PHP_EOL, $a, $b);
}

$top = '%s %s';
$x = 1;
echo sprintf($top, $x);
