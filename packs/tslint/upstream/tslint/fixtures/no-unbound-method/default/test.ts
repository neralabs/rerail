interface I { m?(): void; }
function f(i: I) {
    i.m!();
    (i.m as any)(1);
    (<any> i.m)(1);
    i.m ? 1 : 2;
    if (i.m) {}
    !i.m;
    i.m["length"];
    return i.m!;
}

