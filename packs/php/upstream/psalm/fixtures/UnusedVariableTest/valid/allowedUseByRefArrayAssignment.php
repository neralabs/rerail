<?php
                    $output_rows = [];

                    $a = function() use (&$output_rows) : void {
                        $output_row = 5;
                        $output_rows[] = $output_row;
                    };
                    $a();

                    print_r($output_rows);