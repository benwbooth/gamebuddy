#!/usr/bin/env python3
"""Camera geometry regression tests; these are not gameplay accuracy claims."""
import unittest
import numpy as np
from spatial_geometry import verify


class GeometryTests(unittest.TestCase):
    def test_nonplanar_3d_camera_motion_with_outliers(self):
        rng = np.random.default_rng(72)
        xyz = rng.uniform([-3, -2, 4], [3, 2, 10], (400, 3))
        angle = .15
        rotation = np.array([[np.cos(angle), 0, np.sin(angle)], [0, 1, 0], [-np.sin(angle), 0, np.cos(angle)]])
        def project(points):
            return (points[:, :2] / points[:, 2:] * 170 + [128, 96]).astype(np.float32)
        p = project(xyz)
        q = project(xyz @ rotation.T + [.8, .1, .1])
        visible = ((p > [0, 0]) & (p < [256, 192]) & (q > [0, 0]) & (q < [256, 192])).all(axis=1)
        p, q = p[visible], q[visible]
        q[:40] = rng.uniform([0, 0], [256, 192], (40, 2))
        result = verify(p, q, 'three_d')
        self.assertIsNotNone(result)
        self.assertEqual(result['geometry'], 'epipolar')
        self.assertGreater(result['inliers'], 200)
        self.assertLess(result['residual'], .1)
        # The 3D path deliberately supplies no usable 2D map projection.
        self.assertEqual(result['transform']['tx'], 0)

    def test_unrelated_3d_correspondences_abstain(self):
        rng = np.random.default_rng(7)
        p = rng.uniform([0, 0], [256, 192], (200, 2)).astype(np.float32)
        q = rng.uniform([0, 0], [256, 192], (200, 2)).astype(np.float32)
        self.assertIsNone(verify(p, q, 'three_d'))

    def test_scrolling_affine_and_sparse_hud_rejection(self):
        rng = np.random.default_rng(42)
        p = rng.uniform([20, 20], [220, 160], (120, 2)).astype(np.float32)
        result = verify(p, p + [12, -3], 'two_d')
        self.assertIsNotNone(result)
        self.assertAlmostEqual(result['transform']['tx'], 12, places=3)
        hud = p * [.8, .02]
        self.assertIsNone(verify(hud.astype(np.float32), (hud + [12, 0]).astype(np.float32), 'two_d'))


if __name__ == '__main__':
    unittest.main()
