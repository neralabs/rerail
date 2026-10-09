<?php
                    function foo() : string {
                        return "hello";
                    }

                    /** @var string */
                    $a = foo();

                    echo $a;