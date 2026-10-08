<?php
                    /**
                     * @param array<string, string> $arr0
                     * @param array<string, string> $arr1
                     * @param array<string, string> $arr2
                     * @return void
                     */
                    function parp(array $arr0, array $arr1, array $arr2) {
                        $arr3 = $arr0;

                        foreach ($arr1 as $a) {
                            echo $a;
                            $arr3 = $arr3 + $arr2;
                        }

                        if ($arr3) {}
                    }