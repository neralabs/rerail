
            // doesn't report _x
            let _y, u;
            _y = 1;
            [_y, u] = dqq;
            u;

            // doesn't report _a
            let _x, b;
            _x = 1;
            ({_x, ...b } = dqq);
            b;
            