<?php
                    $a = "hello";
                    function example() : void {
                        global $a;
                        echo $a;
                        $a = "hello";
                    }
                    example();