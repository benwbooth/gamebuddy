"""Scoring and fusion checks: independent labels must constrain acceptance."""
import importlib.util
from pathlib import Path
import unittest

from map_retrieval import hybrid_candidates

spec = importlib.util.spec_from_file_location("benchmark", Path(__file__).with_name("benchmark-map-retrieval.py"))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class RetrievalTests(unittest.TestCase):
    def test_labels_are_explicit_and_bounds_valid(self):
        manifest = {"capture_origin":"independent","label_provenance":"manual",
                    "frames":[{"path":"frame.png"}]}
        with self.assertRaises(ValueError):
            benchmark.validate_manifest(manifest)
        manifest["frames"][0]["expected"] = {"map_id":"eolis","center_bounds":[.5,0,.2,1]}
        with self.assertRaises(ValueError):
            benchmark.validate_manifest(manifest)
        manifest["frames"][0]["expected"] = None
        benchmark.validate_manifest(manifest)

    def test_fixed_budget_and_deduplication(self):
        self.assertEqual(hybrid_candidates([1,2,3,4], [3,5,1,6], 5), [1,3,2,5,4])
        self.assertEqual(hybrid_candidates([1], [1,2,3], 12), [1,2,3])

    def test_invalid_budget(self):
        with self.assertRaises(ValueError):
            hybrid_candidates([1], [2], 0)

    def test_region_constrains_correct_map(self):
        expected = {"map_id":"eolis", "center_bounds":[.1,.2,.2,.7]}
        prediction = {"map_id":"eolis", "polygon":[[.8,.3],[.9,.3],[.9,.6],[.8,.6]]}
        self.assertEqual(benchmark.score_prediction(prediction, expected), "wrong")
        prediction["polygon"] = [[.1,.3],[.2,.3],[.2,.6],[.1,.6]]
        self.assertEqual(benchmark.score_prediction(prediction, expected), "correct")

    def test_negative_and_missing_are_distinct(self):
        self.assertEqual(benchmark.score_prediction(None, None), "correct_abstention")
        self.assertEqual(benchmark.score_prediction(None, {"map_id":"eolis"}), "miss")
        self.assertEqual(benchmark.score_prediction({"map_id":"eolis"}, None), "wrong")

    def test_shortlist_requires_matching_region(self):
        tiles = {1:{"map_id":"eolis","rect":[800,0,200,100],"map_size":[1000,100]},
                 2:{"map_id":"other","rect":[100,0,200,100],"map_size":[1000,100]}}
        expected = {"map_id":"eolis","center_bounds":[.1,.2,.2,.7]}
        self.assertFalse(benchmark.shortlist_hit([1,2], tiles, expected))
        tiles[1]["rect"] = [100,0,200,100]
        self.assertTrue(benchmark.shortlist_hit([1], tiles, expected))

    def test_duplicate_frames_do_not_inflate_unique_image_results(self):
        def frame(digest, outcome):
            return {"sha256": digest, "expected": {"map_id": "eolis"},
                    "methods": {name: {"outcome": outcome, "shortlist_hit": True,
                                       "verified_candidate_hit": outcome == "correct"}
                                for name in benchmark.METHODS}}
        good, missed = frame("first", "correct"), frame("second", "miss")
        frames = [good] * 12 + [missed]
        self.assertEqual(benchmark.summarize(frames)["dino"]["counts"]["correct"], 12)
        unique = benchmark.unique_image_summary(frames)
        self.assertEqual(unique["images"], 2)
        self.assertEqual(unique["summary"]["dino"]["counts"]["correct"], 1)
        conflict = frame("first", "correct")
        conflict["expected"] = None
        with self.assertRaises(ValueError):
            benchmark.unique_image_summary([good, conflict])


if __name__ == "__main__":
    unittest.main()
