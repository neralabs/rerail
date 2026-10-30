
    it("should fail 'expectTypeOf' without 'typecheck' enabled", () => {
     expectTypeOf({ a: 1 }).toEqualTypeOf<{ a: number }>()
    });
    