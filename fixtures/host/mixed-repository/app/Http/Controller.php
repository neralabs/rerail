<?php

namespace App\Http;

final class Controller
{
    public function show(string $title): string
    {
        return escape_html($title);
    }
}
