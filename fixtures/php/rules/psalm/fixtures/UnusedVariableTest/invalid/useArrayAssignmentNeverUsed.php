<?php
                    $data = [];

                    return function () use ($data) {
                        $data[] = 1;
                    };