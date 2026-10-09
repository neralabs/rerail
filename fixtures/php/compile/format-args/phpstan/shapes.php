<?php

declare(strict_types=1);

/** @param list<string> $names */
function shapes(string $name, int $count, float $ratio, array $names): string
{
    $line = sprintf(
        '%s: %d',
        sprintf('[%s]', $name, $count),
        $count,
    );
    $line .= sprintf(
        '%1$s has %2$d items (%3$.2f%%)',
        $name,
        $count,
    );
    $line .= sprintf(
        '%-10s|%\'.10d|%+.1e',
        $name, // the label
        $count,
        $ratio,
        $count,
    );
    $line .= sprintf('%1$s %2$s %1$s', $name, $count);
    $line .= sprintf('%1$s %2$s %1$s', $name);
    $line .= sprintf('%2$s %1$s', $name, $count, $ratio);
    $line .= sprintf('%u%%', $count, $count);
    $line .= sprintf('%05.1f', $ratio);
    $line .= sprintf('%05.1f %s', $ratio);
    $line .= sprintf('%c%c', 65);
    $line .= sprintf('%x %X %o %b', $count, $count, $count);
    $line .= vsprintf('%2$s %1$s', [$name, $count]);
    $line .= vsprintf('%2$s %1$s', [$name]);
    $line .= vsprintf('%2$s %1$s', [1 => $name, 0 => $count, 2 => $ratio]);
    printf("%s\t%s\n", $name);
    printf("%s\t%s\n", $name, $count);
    printf('%s', $name, $count, $ratio);
    $line .= sprintf(
        '%s',
        sprintf('[%s]', ...$names),
    );
    $line .= sprintf(
        $name,
        sprintf('%s', $name, $count),
    );
    return $line;
}
