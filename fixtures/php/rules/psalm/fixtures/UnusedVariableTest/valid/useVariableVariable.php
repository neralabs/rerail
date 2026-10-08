<?php
                    $variables = ["a" => "b", "c" => "d"];

                    foreach ($variables as $name => $value) {
                        ${$name} = $value;
                    }