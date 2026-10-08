<?php
                    /** @param array<string|int> $arr */
                    function foo(array $arr) : string {
                        /** @var string $val */
                        foreach ($arr as &$val) {
                            $val = urlencode($val);
                        }
                        return implode("/", $arr);
                    }