<?php
                    function foo(int $i, string $alias) : void {
                        echo rand(0, 1) ? $i++ : $alias;
                        echo $i;
                    }