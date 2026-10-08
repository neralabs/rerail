<?php
                    $a = $b = 1;
                    $c = &$a;
                    $c = &$b;
                    $c = 2;

                    echo $a + $b + $c;
                