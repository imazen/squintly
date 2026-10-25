#!/usr/bin/env python3
"""As sim_free_levels.py, but fitting a smooth 3-parameter curve per ladder instead of free per-level values.

Curve (zero at the source, convex start, steady rise): d(l) = a * (softplus(b (l - c)) - softplus(-b c)).
Truth: that curve fitted to the AIC-3 boosted refit medians, times a unit factor. Also: detection-only designs (source
against each level, as in a flicker test) for comparison. Planning simulation, not data.
"""
import numpy as np
from scipy.optimize import minimize, least_squares
from scipy.stats import norm

K = norm.ppf(0.75)
BASE = np.array([0.0, 0.18, 0.50, 1.15, 1.65, 2.19, 2.60, 2.97, 3.29, 3.71, 4.09])
L = np.arange(11.0)
sp = lambda x: np.logaddexp(0, x)
curve = lambda p, l: p[0] * (sp(p[1] * (l - p[2])) - sp(-p[1] * p[2]))
true_p = least_squares(lambda p: curve(p, L) - BASE, [0.5, 1.0, 2.0], bounds=([1e-3, 1e-2, -5], [50, 20, 12])).x
print("curve fitted to BASE:", np.round(true_p, 3), "max error", np.round(np.max(np.abs(curve(true_p, L) - BASE)), 3))

def design(kind):
    if kind == "ladder":
        return [(l, 0) for l in range(1, 11)] + [(l + 1, l) for l in range(1, 10)] + [(l + 2, l) for l in range(1, 9)]
    if kind == "detection":
        return [(l, 0) for l in range(1, 11)]

def fit_curve(pairs, wa, wb):
    ia = np.array([p[0] for p in pairs]); ib = np.array([p[1] for p in pairs])
    def nll(q):
        # Nelder-Mead wanders into huge exponents; those points just score badly.
        with np.errstate(over="ignore", invalid="ignore"):
            p = np.r_[np.exp(q[0]), np.exp(q[1]), q[2]]
            d = curve(p, L)
            dd = K * (d[ia] - d[ib])
            return -(wa @ norm.logcdf(dd) + wb @ norm.logcdf(-dd))
    best = None
    for start in ([np.log(0.5), 0.0, 2.0], [np.log(1.0), np.log(0.5), 1.0], [np.log(0.3), np.log(2.0), 3.0]):
        r = minimize(nll, start, method="Nelder-Mead", options={"maxiter": 4000, "xatol": 1e-6, "fatol": 1e-8})
        if best is None or r.fun < best.fun:
            best = r
    q = best.x
    return curve(np.r_[np.exp(q[0]), np.exp(q[1]), q[2]], L)

def crossing(d, t=1.0):
    fine = np.linspace(0, 10, 2001)
    v = np.interp(fine, L, d)
    k = int(np.argmax(v >= t))
    return fine[k] if v[k] >= t else np.nan

rng = np.random.default_rng(11)
print("unit factor | design | answers per ladder | SE of values with d<=3 (median) | SE of 1-JND threshold (JND) | SE of 0.5-JND threshold (JND)")
for factor in [1.0, 1.5]:
    truth = curve(true_p, L) * factor
    slope1 = np.interp(crossing(truth), L, np.gradient(truth)); slope05 = np.interp(crossing(truth, 0.5), L, np.gradient(truth))
    for kind in ["ladder", "detection"]:
        pairs = design(kind)
        ia = np.array([p[0] for p in pairs]); ib = np.array([p[1] for p in pairs])
        p_true = 0.01 + 0.98 * norm.cdf(K * (truth[ia] - truth[ib]))
        for n in [100, 200, 400]:
            m = np.maximum(np.round(np.full(len(pairs), n / len(pairs))), 1).astype(int)
            est, t1, t05 = [], [], []
            for _ in range(200):
                a = rng.binomial(m, p_true).astype(float)
                d = fit_curve(pairs, a, m - a)
                est.append(d); t1.append(crossing(d)); t05.append(crossing(d, 0.5))
            est = np.array(est)
            se_vals = np.median(est[:, (truth <= 3) & (L > 0)].std(axis=0))
            print(f"{factor:>4} | {kind:9s} | {n:4d} | {se_vals:.3f} | {np.nanstd(t1) * slope1:.3f} | {np.nanstd(t05) * slope05:.3f}")
