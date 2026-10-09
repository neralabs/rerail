<?php
                    $arr = [];

                    $populator = function(array &$arr): void {
                        $arr[] = 5;
                    };

                    $populator($arr);

                    print_r($arr);