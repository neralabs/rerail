<?php
                    function &foo(): int
                    {
                        /** @var ?int */
                        static $i;
                        if ($i === null) {
                            $i = 0;
                        }
                        return $i;
                    }

                    $bar = foo();
                