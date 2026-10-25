#!/usr/bin/env python3
"""scripts/lib/test_publish_dry_run.py — unit tests for publish_dry_run.py.

Run with:

    python3 -m unittest discover -s scripts/lib -p 'test_*.py' -v

The metadata is a fixture in the shape of `cargo metadata --no-deps`, so no
test here needs the `cargo` binary.
"""

import unittest

import publish_dry_run


def package(name, publish=None):
    return {"id": f"path+file:///repo/crates/{name}#{name}@0.0.1", "name": name, "publish": publish}


def metadata(*packages, members=None):
    ids = [p["id"] for p in packages] if members is None else members
    return {"packages": list(packages), "workspace_members": ids}


class PublishableMembersTests(unittest.TestCase):
    def test_publish_false_is_private_and_the_default_is_publishable(self):
        data = metadata(
            package("lib"),
            package("tool", publish=[]),
            package("registry", publish=["crates-io", "internal"]),
            package("internal", publish=["internal"]),
        )
        self.assertEqual(publish_dry_run.publishable_members(data), ["lib", "registry"])

    def test_a_package_outside_the_workspace_is_not_a_member(self):
        outside = package("dependency")
        data = metadata(package("lib"), outside, members=[package("lib")["id"]])
        self.assertEqual(publish_dry_run.publishable_members(data), ["lib"])


class CompareTests(unittest.TestCase):
    def test_matching_lists_have_no_errors(self):
        data = metadata(package("a"), package("b"), package("c", publish=[]))
        self.assertEqual(publish_dry_run.compare(data, ["b", "a"]), [])

    def test_an_unlisted_publishable_crate_is_reported(self):
        data = metadata(package("a"), package("b"))
        errors = publish_dry_run.compare(data, ["a"])
        self.assertEqual(len(errors), 1)
        self.assertIn("b may be published", errors[0])

    def test_a_listed_private_crate_is_reported(self):
        data = metadata(package("a"), package("b", publish=[]))
        errors = publish_dry_run.compare(data, ["a", "b"])
        self.assertEqual(len(errors), 1)
        self.assertIn("b is in the list", errors[0])

    def test_a_listed_crate_that_does_not_exist_is_reported(self):
        errors = publish_dry_run.compare(metadata(package("a")), ["a", "ghost"])
        self.assertEqual(len(errors), 1)
        self.assertIn("ghost is in the list", errors[0])


if __name__ == "__main__":
    unittest.main()
