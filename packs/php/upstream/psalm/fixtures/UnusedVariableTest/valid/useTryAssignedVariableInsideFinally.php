<?php
                    $var = "";
                    try {
                        if (rand(0, 1)) {
                            throw new \Exception();
                        }
                        $var = "hello";
                    } finally {
                        if ($var !== "") {
                            echo $var;
                        }
                    }