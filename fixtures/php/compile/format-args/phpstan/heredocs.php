<?php

declare(strict_types=1);

function heredocs(int $a, int $b): void
{
    echo sprintf(<<<EOT
        %s and %s
        EOT, $a);
    echo sprintf(<<<EOT
        %s and %s
        EOT, $a, $b);
    echo sprintf(<<<'EOT'
        %s
        EOT, $a, $b);
    echo sprintf(<<<EOT
        \x25s \\%s
        EOT, $a);
    echo sprintf(<<<EOT
        %s {$a}
        EOT, $b);
}
