<?php
                    exec("pwd 2>&1", $output, $returnValue);
                    if ($returnValue === 0) {
                        echo "success";
                    }