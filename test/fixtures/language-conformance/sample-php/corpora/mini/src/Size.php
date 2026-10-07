<?php

namespace Mini;

final class Size
{
    public function of(array $items): int
    {
        return \sizeof($items);
    }
}
