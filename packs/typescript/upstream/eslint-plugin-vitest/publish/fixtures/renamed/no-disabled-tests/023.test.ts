test("foo", () => {
      const vlozi1 = (x: string) => x.toUpperCase();
      const vlozi2 = (y: string) => y.toUpperCase();
      const vlozi3 = (xq: string) => xq.toUpperCase();
      const vlozi4 = (yd: string) => yd.toUpperCase();
      const vlozi5 = (c: string) => c.toUpperCase();
      const length = (x: string) => x.length;
      expect("test").toBe('test');
    });