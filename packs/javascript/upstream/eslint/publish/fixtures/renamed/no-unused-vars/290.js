function iam(v) {
if (v < 2) return 1;
return v * iam(v - 1);}