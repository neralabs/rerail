test("foo", () => {
      const upper1 = (x: string) => x.toUpperCase();
      const upper2 = (y: string) => y.toUpperCase();
      const upper3 = (xy: string) => xy.toUpperCase();
      const upper4 = (yx: string) => yx.toUpperCase();
      const upper5 = (a: string) => a.toUpperCase();
      const length = (x: string) => x.length;
      expect("test").toBe('test');
    });