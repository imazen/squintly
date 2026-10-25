#!/usr/bin/env python3
"""Trial budget for a jnd-ez study with 1-3 careful observers: how many triplet answers one observer needs per
(source, codec) ladder for a given precision. Planning simulation, not data.

Truth: one ladder of 10 levels whose scale values follow the AIC-3 boosted refit medians (0.18 ... 4.09), times a
factor for a finer expert unit. Observer: P(i judged more distorted than j) = lapse/2 + (1 - lapse) Phi(0.6745 (d_i - d_j)).
Designs: 'ladder' = source-vs-each-level, adjacent and skip-one pairs (27 pairs), answers spread evenly;
'informed' = the same pairs, answers allocated in proportion to each pair's Fisher information at the truth (an upper
bound for adaptive sampling). Fit: Case V maximum likelihood per level, source at 0, weak prior sd 10, no lapse term.
Reports, over 300 simulated observers: SE of the level values at true d <= 3, and SE of the threshold (the level where d
crosses 1, in units of d's local slope, i.e. in JND).
"""
import numpy as np
from scipy.optimize import minimize
from scipy.stats import norm

K = norm.ppf(0.75)
BASE = np.array([0.0, 0.18, 0.50, 1.15, 1.65, 2.19, 2.60, 2.97, 3.29, 3.71, 4.09])
pairs = [(0, l) for l in range(1, 11)] + [(l, l + 1) for l in range(1, 10)] + [(l, l + 2) for l in range(1, 9)]
ia = np.array([p[1] for p in pairs]); ib = np.array([p[0] for p in pairs])  # "more distorted" side first


def fit(wa, wb):
    def nll(z):
        s = np.r_[0.0, z]
        dd = K * (s[ia] - s[ib])
        la, lb = norm.logcdf(dd), norm.logcdf(-dd)
        ra, rb = np.exp(norm.logpdf(dd) - la), np.exp(norm.logpdf(dd) - lb)
        gc = -K * (wa * ra - wb * rb)
        g = np.zeros(11); np.add.at(g, ia, gc); np.add.at(g, ib, -gc)
        return -(wa @ la + wb @ lb) + z @ z / 200.0, g[1:] + z / 100.0
    return np.r_[0.0, minimize(nll, np.linspace(0.3, 3, 10), jac=True, method="L-BFGS-B").x]


def crossing(d, t=1.0):
    k = int(np.argmax(d >= t))
    if k == 0 or d[k] == d[k - 1]:
        return np.nan
    return (k - 1) + (t - d[k - 1]) / (d[k] - d[k - 1])


rng = np.random.default_rng(7)
print("unit factor | lapse | design | answers per ladder | SE of values with d<=3 (median) | SE of threshold level x local slope (JND)")
for factor in [1.0, 1.5]:
    truth = BASE * factor
    lev_true = crossing(truth)
    slope = np.interp(lev_true, np.arange(11), np.gradient(truth))
    for lapse in [0.02]:
        p_true = lapse / 2 + (1 - lapse) * norm.cdf(K * (truth[ia] - truth[ib]))
        info = (K * norm.pdf(K * (truth[ia] - truth[ib]))) ** 2 / (p_true * (1 - p_true))
        for design in ["ladder", "informed"]:
            alloc = np.full(len(pairs), 1.0 / len(pairs)) if design == "ladder" else info / info.sum()
            for n in [100, 200, 400, 800]:
                est, thr = [], []
                for _ in range(300):
                    m = np.maximum(np.round(alloc * n), 1).astype(int)
                    a = rng.binomial(m, p_true).astype(float)
                    d = fit(a, m - a)
                    est.append(d); thr.append(crossing(d))
                est = np.array(est); thr = np.array(thr)
                se_vals = np.median(est[:, truth <= 3].std(axis=0)[1:]) if (truth <= 3).sum() > 1 else np.nan
                se_thr = np.nanstd(thr) * slope
                print(f"{factor:>4} | {lapse:.2f} | {design:8s} | {n:4d} | {se_vals:.3f} | {se_thr:.3f}")
