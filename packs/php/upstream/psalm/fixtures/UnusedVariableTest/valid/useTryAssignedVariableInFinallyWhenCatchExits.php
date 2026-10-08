<?php
                    /**
                     * @return resource
                     */
                    function getStream() {
                        throw new \Exception();
                    }

                    $stream = null;

                    try {
                        $stream = getStream();
                        \file_put_contents("./foobar", $stream);
                    } catch (\Exception $e) {
                        throw new \Exception("Something went wrong");
                    } finally {
                        if ($stream) {
                            \fclose($stream);
                        }
                    }