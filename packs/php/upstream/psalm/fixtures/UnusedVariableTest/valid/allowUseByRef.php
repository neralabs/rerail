<?php
                    function foo(array $data) : array {
                        $output = [];

                        array_map(
                            function (array $row) use (&$output) {
                                $output = $row;
                            },
                            $data
                        );

                        return $output;
                    }