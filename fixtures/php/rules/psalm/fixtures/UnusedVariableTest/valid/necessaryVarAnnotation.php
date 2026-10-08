<?php
                    function foo(array $arr) : void {
                        /** @var int $key */
                        foreach ($arr as $key => $_) {
                            echo $key;
                        }
                    }