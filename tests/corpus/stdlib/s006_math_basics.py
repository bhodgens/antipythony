# Cites: S-05-4 (spec 05 §5.3.1 row math), anchor #4, C-15/C-16, S-06-18
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import math
names = ["acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "cbrt",
         "ceil", "comb", "copysign", "cos", "cosh", "degrees", "dist", "e",
         "erf", "erfc", "exp", "exp2", "expm1", "fabs", "factorial", "floor",
         "fma", "fmod", "frexp", "fsum", "gamma", "gcd", "hypot", "inf",
         "isclose", "isfinite", "isinf", "isnan", "isqrt", "lcm", "ldexp",
         "lgamma", "log", "log10", "log1p", "log2", "modf", "nan",
         "nextafter", "perm", "pi", "pow", "prod", "radians", "remainder",
         "sin", "sinh", "sqrt", "sumprod", "tan", "tanh", "tau", "trunc",
         "ulp"]
print(len(names), all(hasattr(math, n) for n in names))
print(not any(n in dir(math) for n in ("__file__", "_file")))  # S-05-3 native
# Values are pinned, not expressions (spec 05 math row).
print(math.isqrt(10**20), math.gcd(12, 18), math.pi, math.floor(2.7))
print(repr(math.tau), repr(math.e), repr(math.inf), repr(math.nan),
      repr(-math.inf))
print(repr(math.sqrt(2)), repr(math.pow(2, 0.5)), repr(math.exp(1)),
      repr(math.log(math.e)), repr(math.log2(8)), repr(math.log10(1000)),
      repr(math.log1p(0)), repr(math.expm1(0)), repr(math.cbrt(27)),
      repr(math.exp2(3)))
print(repr(math.sin(0)), repr(math.cos(0)), repr(math.tan(0)),
      repr(math.atan2(1, 1)), repr(math.degrees(math.pi)),
      repr(math.radians(180)), repr(math.asin(1)))
print(math.comb(5, 2), math.perm(5, 2), math.factorial(5), math.gcd(0, 7),
      math.lcm(4, 6), math.prod([2, 3]), math.prod(range(1, 6)))
print(math.dist([1, 2], [4, 6]), math.dist([0], [0]), math.hypot(3, 4))
print(math.fma(2, 3, 4), math.sumprod([1, 2], [3, 4]),
      math.sumprod([1, 2, 3], [4, 5, 6]))
print(repr(math.nextafter(1.0, 2.0)), repr(math.ulp(1.0)))
print(repr(math.remainder(10, 3)), repr(math.fmod(7, 2)), repr(math.fabs(-2)),
      repr(math.ldexp(3, 2)), math.frexp(8.0), math.modf(-3.25))
print(repr(math.fsum([0.1] * 10)), repr(math.gamma(5)), repr(math.erf(1)),
      repr(math.erfc(1)), repr(math.lgamma(5)))
print(math.isclose(0.1 + 0.2, 0.3), math.isfinite(1.0), math.isinf(math.inf),
      math.isnan(math.nan))
print(math.copysign(1, -0.0), repr(math.ceil(-1.5)), math.floor(-1.5),
      math.trunc(-1.5))
print(math.isqrt(2**100), math.comb(100, 50) == 100891344545564193334812497256)
