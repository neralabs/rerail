<?php
                    $rules = [0, 1, 2];
                    $report = ["runs" => []];
                    foreach ($rules as $rule) {
                        $report["runs"][] = $rule;
                    }
                    echo(count($report));