#!/usr/bin/env python3
"""Trials per ladder for one careful observer, adaptive (psi method) against fixed allocation.

Task: forced-choice detection triplet (reference, A, B; one of A/B is the reference itself), so
P(correct | level x) = lapse/2 + (1 - lapse) * Phi(K * d(x)), K = Phi^-1(0.75), d in JND (1 JND = 75% correct).
Model fitted: d(x) = (x / alpha)^beta, lapse fixed at 0.02. Truth: the AIC-3 boosted refit medians (levels 0-10),
linearly interpolated, times a sensitivity factor; deliberately not a power law, so the model is misspecified.
Run: `sim_adaptive.py [seed] [aic3|power]`; `power` makes the truth a power law, so the model is well specified.
Reports, per design and number of answers N, the RMSE of the level where d crosses 0.5, 1 and 2, converted to JND
with the true local slope. Planning simulation, not data.
"""
import sys
import numpy as np
from scipy.stats import norm

K = norm.ppf(0.75)
BASE = np.array([0.0, 0.18, 0.50, 1.15, 1.65, 2.19, 2.60, 2.97, 3.29, 3.71, 4.09])
LAPSE = 0.02
TARGETS = (0.5, 1.0, 2.0)
rng = np.random.default_rng(int(sys.argv[1]) if len(sys.argv) > 1 else 5)

alphas = np.exp(np.linspace(np.log(0.2), np.log(12.0), 70))
betas = np.exp(np.linspace(np.log(0.6), np.log(4.0), 30))
A, B = np.meshgrid(alphas, betas, indexing="ij")
A, B = A.ravel(), B.ravel()


def pc_model(x):  # (len(x), n_theta)
    d = (np.asarray(x, float)[:, None] / A[None, :]) ** B[None, :]
    return LAPSE / 2 + (1 - LAPSE) * norm.cdf(K * d)


TRUTH = sys.argv[2] if len(sys.argv) > 2 else "aic3"


def truth_d(x, factor):
    if TRUTH == "power":  # well specified: alpha 2.75, beta 1.7 (close to BASE near threshold)
        return factor * (np.asarray(x, float) / 2.75) ** 1.7
    return factor * np.interp(x, np.arange(11.0), BASE)


def true_crossing(t, factor):
    fine = np.linspace(0, 10, 100001)
    v = truth_d(fine, factor)
    k = int(np.argmax(v >= t))
    return fine[k], (truth_d(fine[k] + 1e-3, factor) - truth_d(fine[k] - 1e-3, factor)) / 2e-3


def estimate(post):
    # posterior mean of each crossing level x_t = alpha * t^(1/beta)
    return [float(post @ (A * t ** (1 / B))) for t in TARGETS]


def run_psi(grid, n, factor):
    P = pc_model(grid)
    post = np.full(A.size, 1.0 / A.size)
    p_true = LAPSE / 2 + (1 - LAPSE) * norm.cdf(K * truth_d(grid, factor))
    for _ in range(n):
        J = P * post
        pc = J.sum(1)
        Jc = J / pc[:, None]
        Ji = (post - J) / (1 - pc)[:, None]
        with np.errstate(divide="ignore", invalid="ignore"):
            Hc = -np.nansum(Jc * np.log(Jc), 1)
            Hi = -np.nansum(Ji * np.log(Ji), 1)
        k = int(np.argmin(pc * Hc + (1 - pc) * Hi))
        correct = rng.random() < p_true[k]
        post = post * (P[k] if correct else 1 - P[k])
        post /= post.sum()
    return estimate(post)


def run_fixed(levels, n, factor):
    levels = np.asarray(levels, float)
    m = np.full(len(levels), n // len(levels))
    m[: n - m.sum()] += 1
    p_true = LAPSE / 2 + (1 - LAPSE) * norm.cdf(K * truth_d(levels, factor))
    c = rng.binomial(m, p_true)
    P = pc_model(levels)
    ll = (c[:, None] * np.log(P) + (m - c)[:, None] * np.log(1 - P)).sum(0)
    post = np.exp(ll - ll.max())
    post /= post.sum()
    return estimate(post)


dense = np.arange(0.25, 10.01, 0.25)
coarse = np.arange(1.0, 11.0)
REPS = 200
print("factor | design | N | RMSE in JND at d=0.5 | d=1 | d=2 (bias at d=1)")
for factor in (1.0, 2.0):
    tx = [true_crossing(t, factor) for t in TARGETS]
    near = dense[(truth_d(dense, factor) >= 0.25) & (truth_d(dense, factor) <= 3.0)]
    near = near[np.linspace(0, len(near) - 1, 6).round().astype(int)]
    near_c = coarse[(truth_d(coarse, factor) >= 0.25) & (truth_d(coarse, factor) <= 3.0)]
    designs = [("psi, dense grid", lambda n: run_psi(dense, n, factor)),
               ("psi, AIC-3 levels", lambda n: run_psi(coarse, n, factor)),
               ("fixed, 6 levels in 0.25-3 JND", lambda n: run_fixed(near, n, factor)),
               ("fixed, AIC-3 levels in 0.25-3 JND", lambda n: run_fixed(near_c, n, factor))]
    if TRUTH == "power":
        designs = designs[:1] + designs[2:3]
    for name, fn in designs:
        for n in (40, 80, 120, 200):
            est = np.array([fn(n) for _ in range(REPS)])
            err = [(est[:, i] - tx[i][0]) * tx[i][1] for i in range(3)]
            rmse = [float(np.sqrt(np.mean(e ** 2))) for e in err]
            print(f"{factor:>3} | {name:34s} | {n:3d} | {rmse[0]:.3f} | {rmse[1]:.3f} | {rmse[2]:.3f} ({np.mean(err[1]):+.3f})",
                  flush=True)
