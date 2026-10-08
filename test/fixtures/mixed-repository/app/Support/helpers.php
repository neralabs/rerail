<?php

function escape_html(string $value): string
{
    return htmlspecialchars($value, ENT_QUOTES);
}
