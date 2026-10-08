<?php
                    function foo() : array {
                        return ["hello"];
                    }

                    /** @var string $s */
                    foreach (foo() as $s) {
                        echo $s;
                    }