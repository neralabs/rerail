<?php
                    function foo(string $value) : array {
                        $arr = str_getcsv($value);

                        foreach ($arr as &$element) {
                            $element = $element !== null ?: "foo";
                        }

                        return $arr;
                    }