<?php
                    $a = [1, 2, 3];
                    $b = &$a[1];
                    $b = 5;
                    print_r($a);