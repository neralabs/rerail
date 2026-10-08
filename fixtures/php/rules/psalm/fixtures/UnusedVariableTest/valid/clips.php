<?php declare(strict_types=1);
                    function foo(array $clips) : void {
                        /** @psalm-suppress MixedAssignment */
                        foreach ($clips as &$clip) {
                            /** @psalm-suppress MixedArgument */
                            if (!empty($clip)) {
                                $legs = explode("/", $clip);
                                $clip_id = $clip = $legs[1];

                                if ((is_numeric($clip_id) || $clip = (new \Exception($clip_id)))) {}

                                print_r($clips);
                            }
                        }
                    }