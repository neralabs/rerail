<?php
                    $i = 0;
                    $a = function () use (&$i) : void {
                        $i = 1;
                    };
                    $a();
                    echo $i;