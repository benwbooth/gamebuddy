"""Geometry checks shared by the learned matcher and independent camera tests."""
import cv2
import numpy as np


def verify(p, q, dimension, atlas=False):
    if len(p) < 16 or len(p) != len(q):
        return None
    if dimension == "three_d":
        matrix, mask = cv2.findFundamentalMat(p, q, cv2.USAC_MAGSAC, 1.5, 0.999, 10000)
        geometry = "epipolar"
    else:
        matrix, mask = cv2.estimateAffine2D(p, q, method=cv2.RANSAC, ransacReprojThreshold=3, maxIters=2000, confidence=0.999, refineIters=10)
        geometry = "affine"
    if matrix is None or mask is None or not np.isfinite(matrix).all():
        return None
    valid = mask.ravel().astype(bool)
    n = int(valid.sum())
    if n < 16 or n / len(p) < 0.5:
        return None
    coverage = float(np.prod(np.ptp(p[valid], axis=0)) / (256 * 192))
    cells = {(int(x / 64), int(y / 48)) for x, y in p[valid]}
    if coverage < 0.2 or len(cells) < 5:
        return None
    identity_affine = dict(xx=1.0, xy=0.0, tx=0.0, yx=0.0, yy=1.0, ty=0.0)
    if geometry == "affine":
        # Atlas tiles span powers of two and need not have the camera's scale.
        # Keep the tighter camera-to-camera limits for place tracking.
        low, high = (0.45, 2.25) if atlas else (0.65, 1.5)
        if not (low < matrix[0, 0] < high and low < matrix[1, 1] < high and abs(matrix[0, 1]) < 0.25 and abs(matrix[1, 0]) < 0.25):
            return None
        if atlas and not (0.65 < matrix[0, 0] / matrix[1, 1] < 1.55):
            return None
        transform = dict(zip(identity_affine, matrix.ravel().tolist()))
        residual = float(np.linalg.norm(p @ matrix[:, :2].T + matrix[:, 2] - q, axis=1)[valid].mean())
    else:
        transform = identity_affine  # Deliberately unusable for map projection.
        ph = np.column_stack((p, np.ones(len(p))))
        qh = np.column_stack((q, np.ones(len(q))))
        fp, fq = ph @ matrix.T, qh @ matrix
        error = (np.sum(qh * fp, axis=1) ** 2 / np.maximum(1e-9, np.sum(fp[:, :2] ** 2 + fq[:, :2] ** 2, axis=1))) ** 0.5
        residual = float(error[valid].mean())
    score = min(1, n / 60) * 0.5 + min(0.8, coverage) * 0.4 + max(0, 1 - residual / 3) * 0.1
    return dict(transform=transform, inliers=n, matches=len(p), coverage=coverage, residual=residual, score=score, geometry=geometry)
