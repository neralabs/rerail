<?php
                    $step = 0;
                    try {
                        $step = 1; // Unused
                        $step = 2;
                        try {
                            $step = 3;
                            $step = 4;
                        } finally {
                            echo $step;
                        }
                    } finally {
                    }
                