<?php
if (!function_exists('getallheaders')) {
    function getallheaders(): array
    {
        return ['Authorization' => 'electrolytes'];
    }
}