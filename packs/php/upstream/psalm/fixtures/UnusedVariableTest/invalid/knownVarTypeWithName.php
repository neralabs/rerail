<?php
                    function foo() : string {
                        return "hello";
                    }

                    /** @var string $a */
                    $a = foo();

                    echo $a;